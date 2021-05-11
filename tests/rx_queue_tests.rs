mod setup;
use libbpf_sys::XDP_PACKET_HEADROOM;
use rusty_fork::rusty_fork_test;
use std::{thread, time::Duration};
use xdpsock::{
    socket::{SocketConfig, SocketConfigBuilder},
    umem::{Frame, UmemConfig, UmemConfigBuilder},
    xsk::Xsk,
};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfigBuilder::new()
        .frame_count(8)
        .frame_size(2048)
        .fill_queue_size(4)
        .comp_queue_size(4)
        .build()
        .expect("failed to build umem config");

    let socket_config = SocketConfigBuilder::new()
        .tx_queue_size(4)
        .rx_queue_size(4)
        .build()
        .expect("failed to build socket config");

    (Some(umem_config), Some(socket_config))
}

rusty_fork_test! {
    #[test]
    fn rx_queue_consumes_nothing_if_no_tx_and_fill_q_empty() {
        fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
            let n_rx_frames = dev1.rx_frames.len();
            let mut filled_frames = vec![(0, 0, 0); n_rx_frames];
            assert_eq!(dev1.rx_q.consume(2, &mut filled_frames[..2]), 0);

            assert_eq!(
                dev1.rx_q
                    .poll_and_consume(2, &mut filled_frames[..2], 100)
                    .unwrap(),
                0
            );
        }

        let (dev1_umem_config, dev1_socket_config) = build_configs();
        let (dev2_umem_config, dev2_socket_config) = build_configs();

        setup::run_test(
            dev1_umem_config,
            dev1_socket_config,
            dev2_umem_config,
            dev2_socket_config,
            test_fn,
        );
    }
}

rusty_fork_test! {
    #[test]
    fn rx_queue_consume_returns_nothing_if_fill_q_empty() {
        fn test_fn(mut dev1: Xsk, mut dev2: Xsk) {
            let n_rx_frames = dev1.rx_frames.len();
            let mut filled_frames = vec![(0, 0, 0); n_rx_frames];

            assert_eq!(
                unsafe {
                    dev2.tx_q
                        .produce_and_wakeup(&dev2.tx_frames[..4])
                        .unwrap()
                },
                4
            );

            assert_eq!(dev1.rx_q.consume(4, &mut filled_frames[..4]), 0);

            assert_eq!(
                dev1.rx_q
                    .poll_and_consume(4, &mut filled_frames[..4], 100)
                    .unwrap(),
                0
            );
        }

        let (dev1_umem_config, dev1_socket_config) = build_configs();
        let (dev2_umem_config, dev2_socket_config) = build_configs();

        setup::run_test(
            dev1_umem_config,
            dev1_socket_config,
            dev2_umem_config,
            dev2_socket_config,
            test_fn,
        );
    }
}

rusty_fork_test! {
    #[test]
    fn rx_queue_consumes_frame_correctly_after_tx() {
        fn test_fn(mut dev1: Xsk, mut dev2: Xsk) {
            let n_rx_frames = dev1.rx_frames.len();
            let mut filled_frames = vec![(0, 0, 0); n_rx_frames];

            // Add a frame in the dev1 fill queue ready to receive
            let mut rx_frames: Vec<&Frame> = dev1.rx_frames.iter().collect();

            assert_eq!(unsafe { dev1.fill_q.produce(&mut rx_frames[0..1]) }, 1);

            // Data to send from dev2
            let pkt = vec![b'H', b'e', b'l', b'l', b'o'];

            // Write data to UMEM
            unsafe {
                dev2.tx_frames[0].write_to_umem_checked(&pkt[..]).unwrap();
            }

            assert_eq!(dev2.tx_frames[0].len(), 5);

            // Transmit data
            assert_eq!(
                unsafe {
                    dev2.tx_q
                        .produce_and_wakeup(&dev2.tx_frames[0..1])
                        .unwrap()
                },
                1
            );

            // Wait briefly so we don't try to consume too early
            thread::sleep(Duration::from_millis(5));

            // Read on dev1
            let n_frames_filled = dev1.rx_q.consume(1, &mut filled_frames);
            assert_eq!(n_frames_filled, 1);

            // TODO: Dry this
            let rx_frame_offset = dev1.rx_frames[0].addr();
            let frame_size = dev1.umem_config.frame_size();

            for filled_frame in filled_frames[..n_frames_filled].iter() {
                let rx_frame_index =
                    (filled_frame.0 as u32 - rx_frame_offset as u32) / frame_size;

                let rx_frame_addr = filled_frame.0;
                let rx_frame_len = filled_frame.1;
                let rx_frame_options = filled_frame.2;

                dev1.rx_frames[rx_frame_index as usize].set_addr(rx_frame_addr);
                dev1.rx_frames[rx_frame_index as usize].set_len(rx_frame_len);
                dev1.rx_frames[rx_frame_index as usize].set_options(rx_frame_options);
            }

            // Check that the data is correct
            let recvd = unsafe {
                dev1.rx_frames[0]
                    .read_from_umem_checked(dev1.rx_frames[0].len())
                    .unwrap()
            };

            assert_eq!(recvd[..5], pkt[..]);
        }

        let (dev1_umem_config, dev1_socket_config) = build_configs();
        let (dev2_umem_config, dev2_socket_config) = build_configs();

        setup::run_test(
            dev1_umem_config,
            dev1_socket_config,
            dev2_umem_config,
            dev2_socket_config,
            test_fn,
        );
    }
}

rusty_fork_test! {
    #[test]
    fn rx_queue_recvd_packet_offset_after_tx_includes_xdp_and_frame_headroom() {
        fn test_fn(mut dev1: Xsk, mut dev2: Xsk) {
            let n_rx_frames = dev1.rx_frames.len();
            let mut filled_frames = vec![(0, 0, 0); n_rx_frames];
            let rx_frames: Vec<&Frame> = dev1.rx_frames.iter().collect();

            // Add a frame in the dev1 fill queue ready to receive
            assert_eq!(unsafe { dev1.fill_q.produce(&rx_frames[..1]) }, 1);

            // Data to send from dev2
            let pkt = vec![b'H', b'e', b'l', b'l', b'o'];

            // Write data to UMEM
            unsafe {
                dev2.tx_frames[0].write_to_umem_checked(&pkt[..]).unwrap();
            }

            assert_eq!(dev2.tx_frames[0].len(), 5);

            // Transmit data
            assert_eq!(
                unsafe {
                    dev2.tx_q
                        .produce_and_wakeup(&dev2.tx_frames[0..1])
                        .unwrap()
                },
                1
            );

            // Wait briefly so we don't try to consume too early
            thread::sleep(Duration::from_millis(5));

            let n_frames_filled = dev1.rx_q.consume(1, &mut filled_frames);
            assert_eq!(n_frames_filled, 1);

            let rx_frame_offset = dev1.rx_frames[0].addr();
            let frame_size = dev1.umem_config.frame_size();
            for filled_frame in filled_frames[..n_frames_filled].iter() {
                let rx_frame_index =
                    (filled_frame.0 as u32 - rx_frame_offset as u32) / frame_size;

                let rx_frame_addr = filled_frame.0;
                let rx_frame_len = filled_frame.1;
                let rx_frame_options = filled_frame.2;

                dev1.rx_frames[rx_frame_index as usize].set_addr(rx_frame_addr);
                dev1.rx_frames[rx_frame_index as usize].set_len(rx_frame_len);
                dev1.rx_frames[rx_frame_index as usize].set_options(rx_frame_options);
            }

            assert_eq!(dev1.rx_frames[0].len(), 5);

            // Check addr starts where we expect
            assert_eq!(
                dev1.rx_frames[0].addr(),
                (XDP_PACKET_HEADROOM + 512 + 8192) as usize // we're using the second half of the frames for rx
            );
        }

        let (dev2_umem_config, dev2_socket_config) = build_configs();

        // Add to the frame headroom
        let dev1_umem_config = UmemConfigBuilder::new()
            .frame_count(8)
            .frame_size(2048)
            .fill_queue_size(4)
            .comp_queue_size(4)
            .frame_headroom(512).build().expect("failed to build umem config");
        let dev1_socket_config = SocketConfigBuilder::new().tx_queue_size(4).rx_queue_size(4).build().expect("failed to build socket config");

        setup::run_test(
            Some(dev1_umem_config),
            Some(dev1_socket_config),
            dev2_umem_config,
            dev2_socket_config,
            test_fn,
        );
    }
}

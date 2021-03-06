mod setup;
use rusty_fork::rusty_fork_test;
use std::{thread, time::Duration};
use xdpsock::{
    socket::{SocketConfig, SocketConfigBuilder},
    umem::{UmemConfig, UmemConfigBuilder},
    xsk::Xsk,
};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfigBuilder::new()
        .frame_count(8)
        .comp_queue_size(8)
        .build()
        .unwrap();

    let socket_config = SocketConfigBuilder::new().tx_queue_size(4).build().unwrap();

    (Some(umem_config), Some(socket_config))
}

rusty_fork_test! {
    #[test]
    fn comp_queue_consumes_nothing_if_tx_q_unused() {

        fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
            let dev1_frames = dev1.tx_frames;
            let n_tx_frames = dev1_frames.len();
            let mut free_frames = vec![0; n_tx_frames];

            eprintln!("frames[1] = {}", dev1_frames[1].addr());

            let n_free_frames = dev1.comp_q.consume(4, &mut free_frames);
            assert_eq!(n_free_frames, 0);
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
    fn comp_queue_num_frames_consumed_match_those_produced() {
        fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
            let dev1_frames = dev1.tx_frames;
            let n_tx_frames = dev1_frames.len();
            let mut free_frames = vec![0; n_tx_frames];

            assert_eq!(
                unsafe { dev1.tx_q.produce_and_wakeup(&dev1_frames[..2]).unwrap() },
                2
            );

            // Wait briefly so we don't try to consume too early
            thread::sleep(Duration::from_millis(5));

            let n_free_frames = dev1.comp_q.consume(4, &mut free_frames);

            assert_eq!(n_free_frames, 2);
        }

        //thread::sleep(Duration::from_secs(5));

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
    fn comp_queue_addr_of_frames_consumed_match_addr_of_those_produced() {
        fn test_fn(mut dev1: Xsk, _dev2: Xsk) {

            let dev1_frames = dev1.tx_frames;
            let n_tx_frames = dev1_frames.len();
            let mut free_frames = vec![0; n_tx_frames];

            let produced_addrs: Vec<u64> = dev1_frames[2..4].iter().map(|f| f.addr() as u64).collect();

            unsafe {
                dev1.tx_q
                    .produce_and_wakeup(&dev1_frames[2..4])
                    .unwrap()
            };

            // Wait briefly so we don't try to consume too early
            thread::sleep(Duration::from_millis(5));

            let n_free_frames = dev1.comp_q.consume(2, &mut free_frames);
            assert_eq!(n_free_frames, 2);

            // Also ensure that the frame info matches
            assert_eq!(
                free_frames[..n_free_frames as usize], produced_addrs
            );
        }

        //thread::sleep(Duration::from_secs(10));
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

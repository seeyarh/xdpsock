mod setup;

use rusty_fork::rusty_fork_test;
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;

use xsk_rs::{
    socket::{SocketConfig, SocketConfigBuilder},
    umem::{UmemConfig, UmemConfigBuilder},
    xsk::Xsk,
};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfigBuilder::new()
        .frame_count(8)
        .build()
        .expect("failed to build umem config");

    let socket_config = SocketConfigBuilder::new()
        .tx_queue_size(4)
        .build()
        .expect("failed to build socket config");

    (Some(umem_config), Some(socket_config))
}

rusty_fork_test! {
    #[test]
    fn send_recv_test() {
        fn test_fn(mut dev1: Xsk, mut dev2: Xsk) {
            // Add a frame in the dev1 fill queue ready to receive
            assert_eq!(unsafe { dev1.fill_q.produce(&dev1.frame_descs[0..1]) }, 1);

            // Data to send from dev2
            let pkt = vec![b'H', b'e', b'l', b'l', b'o'];

            // Write data to UMEM
            unsafe {
                dev2.frame_descs[0].write_to_umem_checked(&pkt[..]).unwrap();
            }

            assert_eq!(dev2.frame_descs[0].len(), 5);

            // Transmit data
            assert_eq!(
                unsafe {
                    dev2.tx_q
                        .produce_and_wakeup(&dev2.frame_descs[0..1])
                        .unwrap()
                },
                1
            );



            // Wait briefly so we don't try to consume too early
            thread::sleep(Duration::from_millis(5));

            // Read on dev1
            let frames_consumed = dev1.rx_q.consume(&mut dev1.frame_descs[..]);
            assert_eq!(frames_consumed, 1);

            assert_eq!(dev1.frame_descs[0].len(), 5);

            // Check that the data is correct
            let recvd = unsafe {
                dev1.frame_descs[0]
                    .read_from_umem_checked(dev1.frame_descs[0].len())
                    .unwrap()
            };

            assert_eq!(recvd[..5], pkt[..]);
        }

        let (dev1_umem_config, dev1_socket_config) = build_configs();
        let (dev2_umem_config, dev2_socket_config) = build_configs();

        let mut rt = Runtime::new().unwrap();
        rt.block_on(async {
            setup::run_test(
                dev1_umem_config,
                dev1_socket_config,
                dev2_umem_config,
                dev2_socket_config,
                test_fn,
            ).await;
        });
    }
}

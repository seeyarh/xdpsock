mod setup;
use rusty_fork::rusty_fork_test;
use xdpsock::{
    socket::SocketConfig,
    umem::{UmemConfig, UmemConfigBuilder},
    xsk::Xsk,
};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfigBuilder::new()
        .frame_count(16)
        .fill_queue_size(4)
        .build()
        .expect("failed to build umem config");

    (Some(umem_config), None)
}

rusty_fork_test! {
    #[test]
    fn fill_queue_produce_tx_size_frames() {
        fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
            let mut frames = dev1.rx_frames;
            assert_eq!(unsafe { dev1.fill_q.produce(&mut frames[..4]) }, 4);
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
    fn fill_queue_produce_gt_tx_size_frames() {
        fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
            let mut frames = dev1.rx_frames;

            assert_eq!(unsafe { dev1.fill_q.produce(&mut frames[..5]) }, 0);
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
    fn fill_queue_produce_frames_until_full() {
        fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
            let mut frames = dev1.rx_frames;

            assert_eq!(unsafe { dev1.fill_q.produce(&mut frames[..2]) }, 2);
            assert_eq!(unsafe { dev1.fill_q.produce(&mut frames[2..3]) }, 1);
            assert_eq!(unsafe { dev1.fill_q.produce(&mut frames[3..8]) }, 0);
            assert_eq!(unsafe { dev1.fill_q.produce(&mut frames[3..4]) }, 1);
        }

        let (dev1_umem_config, dev1_socket_config) = build_configs();
        let (dev2_umem_config, dev2_socket_config) = build_configs();

        setup::run_test(
            dev1_umem_config,
            dev1_socket_config,
            dev2_umem_config,
            dev2_socket_config,
            test_fn,
        )
    }
}

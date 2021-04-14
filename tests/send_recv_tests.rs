mod setup;

use rusty_fork::rusty_fork_test;
use std::thread;
use std::time::Duration;

use xsk_rs::{socket::SocketConfig, umem::UmemConfig, xsk::Xsk2};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfig::default();
    let socket_config = SocketConfig::default();
    (Some(umem_config), Some(socket_config))
}

#[test]
fn send_recv_test() {
    fn test_fn(mut dev1: Xsk2, mut dev2: Xsk2) {
        // Data to send from dev2
        let pkt = vec![b'H', b'e', b'l', b'l', b'o'];

        let mut recvd_packets = vec![];
        thread::sleep(Duration::from_millis(5));

        dev1.send(&pkt);
        thread::sleep(Duration::from_millis(5));

        for _ in (0..5) {
            if let Some(recvd) = dev2.recv() {
                recvd_packets.push(recvd);
            }
            thread::sleep(Duration::from_millis(50));
        }

        let mut matched = false;
        for recvd_pkt in recvd_packets {
            if pkt == recvd_pkt {
                matched = true;
            }
        }

        assert!(matched);

        dev1.shutdown();
        dev2.shutdown();
    }

    let (dev1_umem_config, dev1_socket_config) = build_configs();
    let (dev2_umem_config, dev2_socket_config) = build_configs();

    setup::run_test_2(
        dev1_umem_config,
        dev1_socket_config,
        dev2_umem_config,
        dev2_socket_config,
        test_fn,
    );
}

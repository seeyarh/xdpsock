mod setup;

use rusty_fork::rusty_fork_test;
use std::thread;
use std::time::Duration;

use xsk_rs::{socket::SocketConfig, umem::UmemConfig, xsk::Xsk};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfig::default();
    let socket_config = SocketConfig::default();
    (Some(umem_config), Some(socket_config))
}

#[test]
fn send_recv_test() {
    fn test_fn(mut dev1: Xsk, mut dev2: Xsk) {
        // Data to send from dev2
        let pkt = vec![b'H', b'e', b'l', b'l', b'o'];

        dev1.send(&pkt).expect("failed to send pkt");
        dev2.start_recv();

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        let mut recvd_packets = vec![];
        recvd_packets.append(&mut dev2.recv());
        thread::sleep(Duration::from_millis(5000));

        recvd_packets.append(&mut dev2.recv());
        thread::sleep(Duration::from_millis(5));

        recvd_packets.append(&mut dev2.recv());
        thread::sleep(Duration::from_millis(5));

        recvd_packets.append(&mut dev2.recv());

        let mut matched = false;
        for recvd_pkt in recvd_packets {
            eprintln!("{:?}", recvd_pkt);
            if pkt == recvd_pkt {
                matched = true;
            }
        }

        assert!(matched);
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

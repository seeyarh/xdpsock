mod setup;

use rusty_fork::rusty_fork_test;
use std::collections::HashSet;
use std::io::Cursor;
use std::thread;
use std::time::Duration;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use xsk_rs::{socket::SocketConfig, umem::UmemConfig, xsk::Xsk2};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfig::default();
    let socket_config = SocketConfig::default();
    (Some(umem_config), Some(socket_config))
}

#[test]
fn send_recv_test() {
    fn test_fn(mut dev1: Xsk2, mut dev2: Xsk2) {
        let pkts_to_send = 100_000;

        let tx_send = dev1.tx_sender().unwrap();
        let rx_recv = dev2.rx_receiver().unwrap();

        let send_handle = thread::spawn(move || {
            for i in 0..pkts_to_send {
                let mut pkt = vec![];
                pkt.write_u64::<LittleEndian>(i).unwrap();
                eprintln!("sending {}", i);
                tx_send.send(pkt).unwrap();
            }
        });

        let recv_handle = thread::spawn(move || rx_recv.iter().collect());
        thread::sleep(Duration::from_millis(50));

        thread::sleep(Duration::from_secs(10));
        send_handle.join().expect("failed to join tx handle");
        eprintln!("send done");

        dev1.shutdown();
        dev2.shutdown();
        let recvd_pkts: Vec<Vec<u8>> = recv_handle.join().expect("failed to join recv handle");

        let recvd_nums: HashSet<u64> = recvd_pkts
            .into_iter()
            .map(|x| {
                let mut rdr = Cursor::new(x);
                rdr.read_u64::<LittleEndian>().unwrap()
            })
            .collect();

        let expected_recvd_nums: Vec<u64> = (0..pkts_to_send).into_iter().collect();

        for n in expected_recvd_nums.iter() {
            assert!(recvd_nums.contains(n));
        }
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

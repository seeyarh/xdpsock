mod setup;

use rusty_fork::rusty_fork_test;
use std::collections::HashSet;
use std::io::Cursor;
use std::thread;
use std::time::{Duration, Instant};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use etherparse::{
    IpHeader, PacketBuilder, PacketBuilderStep, ReadError, TransportHeader, UdpHeader,
};

use xsk_rs::{
    socket::SocketConfig,
    umem::UmemConfig,
    xsk::{ParsedPacket, Xsk2},
};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfig::default();
    let socket_config = SocketConfig::default();
    (Some(umem_config), Some(socket_config))
}

fn generate_pkt(pkt_builder: PacketBuilderStep<UdpHeader>, n: u64) -> Vec<u8> {
    let mut payload = vec![];
    payload.write_u64::<LittleEndian>(n).unwrap();
    //get some memory to store the result
    let mut result = Vec::<u8>::with_capacity(pkt_builder.size(payload.len()));

    //serialize
    pkt_builder
        .write(&mut result, &payload)
        .expect("failed to build packet");
    result
}

const SRC_IP: [u8; 4] = [192, 168, 69, 1];
const DST_IP: [u8; 4] = [192, 168, 69, 2];

const SRC_PORT: u16 = 1234;
const DST_PORT: u16 = 4321;

fn filter_pkt(pkt: &ParsedPacket) -> bool {
    let mut ip_match = false;
    let mut transport_match = false;
    if let Some(ref ip) = pkt.ip {
        if let IpHeader::Version4(ipv4) = ip {
            ip_match = (ipv4.source == SRC_IP) && (ipv4.destination == DST_IP);
        }
    }

    if let Some(ref transport) = pkt.transport {
        if let TransportHeader::Udp(udp) = transport {
            transport_match = (udp.source_port == SRC_PORT) && (udp.destination_port == DST_PORT);
        }
    }

    ip_match && transport_match
}

#[test]
fn send_recv_test() {
    fn test_fn(mut dev1: Xsk2, mut dev2: Xsk2) {
        let pkts_to_send = 1_048_576;

        let tx_send = dev1.tx_sender().unwrap();
        let rx_recv = dev2.rx_receiver().unwrap();

        let recv_handle = thread::spawn(move || {
            let mut recvd_nums: HashSet<u64> = HashSet::new();
            for pkt in rx_recv.iter() {
                let pkt = pkt.expect("failed to read pkt");
                if filter_pkt(&pkt) {
                    let payload = pkt.payload.expect("no payload");
                    let mut rdr = Cursor::new(&payload[0..8]);
                    let n = rdr.read_u64::<LittleEndian>().unwrap();
                    recvd_nums.insert(n);
                }
            }
            recvd_nums
        });

        thread::sleep(Duration::from_millis(50));

        let send_handle = thread::spawn(move || {
            let start = Instant::now();
            for i in 0..pkts_to_send {
                let pkt_builder = PacketBuilder::ethernet2([0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0])
                    .ipv4(SRC_IP, DST_IP, 20)
                    .udp(SRC_PORT, DST_PORT);
                let pkt_with_payload = generate_pkt(pkt_builder, i);
                tx_send.send(pkt_with_payload).unwrap();
                if i % 1000 == 0 {
                    thread::sleep(Duration::from_micros(1));
                }
            }
            let duration = start.elapsed();
            eprintln!("send time is: {:?}", duration);
        });

        thread::sleep(Duration::from_secs(1));
        send_handle.join().expect("failed to join tx handle");
        eprintln!("send done");

        dev1.shutdown();
        dev2.shutdown();
        let recvd_nums = recv_handle.join().expect("failed to join recv handle");

        let expected_recvd_nums: Vec<u64> = (0..pkts_to_send).into_iter().collect();

        let mut n_missing = 0;
        for n in expected_recvd_nums.iter() {
            if !recvd_nums.contains(n) {
                eprintln!("missing {}", n);
                n_missing += 1;
            }
        }
        assert_eq!(n_missing, 0);
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

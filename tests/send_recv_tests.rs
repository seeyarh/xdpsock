mod setup;

use rusty_fork::rusty_fork_test;
use std::collections::HashSet;
use std::error::Error;
use std::io::Cursor;
use std::net::Ipv4Addr;
use std::thread;
use std::time::{Duration, Instant};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use etherparse::{
    InternetSlice, PacketBuilder, PacketBuilderStep, SlicedPacket, TransportSlice, UdpHeader,
};

use xdpsock::{
    socket::{SocketConfig, SocketConfigBuilder},
    umem::{UmemConfig, UmemConfigBuilder},
    xsk::{Xsk2, MAX_PACKET_SIZE},
};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfigBuilder::new()
        .frame_count(8192)
        .comp_queue_size(4096)
        .fill_queue_size(4096)
        .build()
        .unwrap();

    let socket_config = SocketConfigBuilder::new()
        .tx_queue_size(4096)
        .rx_queue_size(4096)
        .build()
        .unwrap();

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

#[derive(Debug, Clone)]
struct Filter {
    src_ip: [u8; 4],
    src_port: u16,
    dest_ip: [u8; 4],
    dest_port: u16,
}

impl Filter {
    fn new(
        src_ip: [u8; 4],
        src_port: u16,
        dest_ip: [u8; 4],
        dest_port: u16,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            src_ip,
            src_port,
            dest_ip,
            dest_port,
        })
    }
}

fn filter_pkt(parsed_pkt: &SlicedPacket, filter: &Filter) -> bool {
    let mut ip_match = false;
    let mut transport_match = false;
    if let Some(ref ip) = parsed_pkt.ip {
        if let InternetSlice::Ipv4(ipv4) = ip {
            ip_match = (ipv4.source() == filter.src_ip) && (ipv4.destination() == filter.dest_ip);
        }
    }

    if let Some(ref transport) = parsed_pkt.transport {
        if let TransportSlice::Udp(udp) = transport {
            transport_match = (udp.source_port() == filter.src_port)
                && (udp.destination_port() == filter.dest_port);
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

        let filter = Filter::new(SRC_IP, SRC_PORT, DST_IP, DST_PORT).unwrap();

        let recv_handle = thread::spawn(move || {
            let mut recvd_nums: HashSet<u64> = HashSet::new();
            for (pkt, len) in rx_recv.iter() {
                match SlicedPacket::from_ethernet(&pkt[..len]) {
                    Ok(pkt) => {
                        if filter_pkt(&pkt, &filter) {
                            let mut rdr = Cursor::new(&pkt.payload[0..8]);
                            let n = rdr.read_u64::<LittleEndian>().unwrap();
                            recvd_nums.insert(n);
                        }
                    }
                    Err(e) => log::warn!("failed to parse packet {:?}", e),
                }
            }
            recvd_nums
        });

        // give the receiver a chance to get going
        thread::sleep(Duration::from_millis(50));

        let send_handle = thread::spawn(move || {
            let start = Instant::now();
            for i in 0..pkts_to_send {
                let pkt_builder = PacketBuilder::ethernet2([0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0])
                    .ipv4(SRC_IP, DST_IP, 20)
                    .udp(SRC_PORT, DST_PORT);
                let pkt_with_payload = generate_pkt(pkt_builder, i);

                let mut packet: [u8; MAX_PACKET_SIZE] = [0; MAX_PACKET_SIZE];
                let l = std::cmp::min(MAX_PACKET_SIZE, pkt_with_payload.len());
                let packet_slice = &mut packet[..l];
                packet_slice.copy_from_slice(&pkt_with_payload[..l]);

                tx_send.send((packet, pkt_with_payload.len())).unwrap();
            }
            let duration = start.elapsed();
            eprintln!("send time is: {:?}", duration);
        });

        send_handle.join().expect("failed to join tx handle");

        let dev1_tx_stats = dev1.shutdown_tx().expect("failed to shutdown tx");
        eprintln!("dev1 tx_stats = {:?}", dev1_tx_stats);
        eprintln!("dev1 tx duration = {:?}", dev1_tx_stats.duration());
        eprintln!("dev1 tx pps = {:?}", dev1_tx_stats.pps());

        let dev1_rx_stats = dev1.shutdown_rx().expect("failed to shut down rx");
        eprintln!("dev1 rx_stats = {:?}", dev1_rx_stats);

        let dev2_tx_stats = dev2.shutdown_tx().expect("failed to shut down tx");
        eprintln!("dev2 tx_stats = {:?}", dev2_tx_stats);

        let dev2_rx_stats = dev2.shutdown_rx().expect("failed to shut down rx");
        eprintln!("dev2 rx_stats = {:?}", dev2_rx_stats);
        eprintln!("dev2 rx duration = {:?}", dev2_rx_stats.duration());
        eprintln!("dev2 rx pps = {:?}", dev2_rx_stats.pps());

        assert_eq!(dev1_tx_stats.pkts_tx, pkts_to_send);

        // we can receive extra packets due to random traffic
        assert!(dev2_rx_stats.pkts_rx >= pkts_to_send);

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

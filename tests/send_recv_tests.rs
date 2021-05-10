mod setup;

use std::collections::HashSet;
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use byteorder::{ByteOrder, LittleEndian};
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

fn generate_pkt(
    mut pkt: &mut [u8],
    payload: &mut [u8],
    pkt_builder: PacketBuilderStep<UdpHeader>,
) -> usize {
    let len = pkt_builder.size(payload.len());
    pkt_builder
        .write(&mut pkt, payload)
        .expect("failed to build packet");
    len
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
    fn test_fn(mut dev1: Xsk2<'static>, mut dev2: Xsk2<'static>) {
        let pkts_to_send = 1_000_000 as u64;

        let filter = Filter::new(SRC_IP, SRC_PORT, DST_IP, DST_PORT).unwrap();

        let send_done = Arc::new(AtomicBool::new(false));
        let send_done_rx = send_done.clone();

        let rx_timeout = Duration::from_secs(5);

        eprintln!("starting receiver");
        let recv_handle = thread::spawn(move || {
            let mut pkt: [u8; MAX_PACKET_SIZE] = [0; MAX_PACKET_SIZE];
            let mut matched_recvd_pkts = 0;
            let mut recvd_nums: HashSet<u64> = HashSet::with_capacity(pkts_to_send as usize);
            let mut send_done_time: Option<Instant> = None;
            let start = Instant::now();

            while matched_recvd_pkts != pkts_to_send {
                if send_done_rx.load(Ordering::Relaxed) {
                    if let Some(send_done_time) = send_done_time {
                        if send_done_time.elapsed() > rx_timeout {
                            eprintln!("recv ending after timeout");
                            break;
                        }
                    } else {
                        send_done_time = Some(Instant::now());
                    }
                }

                let len_recvd = dev1.rx.recv(&mut pkt[..]);
                if len_recvd > 0 {
                    match SlicedPacket::from_ethernet(&pkt[..len_recvd]) {
                        Ok(pkt) => {
                            if filter_pkt(&pkt, &filter) {
                                let n = LittleEndian::read_u64(&pkt.payload[..8]);
                                recvd_nums.insert(n);
                                matched_recvd_pkts += 1;
                            }
                        }
                        Err(e) => log::warn!("failed to parse packet {:?}", e),
                    }
                }
                recvd_nums.insert(1);

                for b in &mut pkt[..len_recvd] {
                    *b = 0;
                }
            }

            let duration = start.elapsed();
            eprintln!("receive time is: {:?}", duration);
            (dev1.rx.stats(), recvd_nums)
        });

        // give the receiver a chance to get going
        thread::sleep(Duration::from_millis(50));

        eprintln!("starting sender");
        let send_handle = thread::spawn(move || {
            let mut pkt: [u8; MAX_PACKET_SIZE] = [0; MAX_PACKET_SIZE];
            let mut payload: [u8; 8] = [0; 8];

            let start = Instant::now();
            for i in 0..pkts_to_send {
                //thread::sleep(Duration::from_millis(1));
                let pkt_builder = PacketBuilder::ethernet2([0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0])
                    .ipv4(SRC_IP, DST_IP, 20)
                    .udp(SRC_PORT, DST_PORT);

                LittleEndian::write_u64(&mut payload, i);
                let len_pkt = generate_pkt(&mut pkt[..], &mut payload[..], pkt_builder);

                while let Err(_) = dev2.tx.send(&pkt[..len_pkt]) {}
            }
            send_done.store(true, Ordering::Relaxed);
            let duration = start.elapsed();
            eprintln!("send time is: {:?}", duration);
            dev2.tx.stats()
        });

        let tx_stats = send_handle.join().expect("failed to join tx handle");
        eprintln!("send done");

        /*
        assert_eq!(dev1_tx_stats.pkts_tx, pkts_to_send);

        // we can receive extra packets due to random traffic
        assert!(dev2_rx_stats.pkts_rx >= pkts_to_send);
        */

        let (rx_stats, recvd_nums) = recv_handle.join().expect("failed to join recv handle");
        eprintln!("recv done");

        eprintln!("tx stats {:?}", tx_stats);
        eprintln!("rx stats {:?}", rx_stats);

        let expected_recvd_nums: Vec<u64> = (0..pkts_to_send).into_iter().collect();

        let mut n_missing = 0;
        for n in expected_recvd_nums.iter() {
            if !recvd_nums.contains(n) {
                //eprintln!("missing {}", n);
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

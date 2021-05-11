use std::collections::HashSet;
use std::error::Error;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use byteorder::{ByteOrder, LittleEndian};
use clap::Clap;
use etherparse::{
    InternetSlice, PacketBuilder, PacketBuilderStep, SlicedPacket, TransportSlice, UdpHeader,
};

use xdpsock::{
    socket::{BindFlags, SocketConfigBuilder, XdpFlags},
    umem::UmemConfigBuilder,
    xsk::{Xsk2, MAX_PACKET_SIZE},
};

#[derive(Clap, Debug, Clone)]
enum Mode {
    Tx,
    Rx,
}

/// Send or Receive UDP packets on the specified interface
#[derive(Debug, Clone, Clap)]
#[clap(version = "1.0", author = "Collins Huff")]
struct Opts {
    /// interface name
    #[clap(short, long)]
    dev: String,

    /// source MAC address
    #[clap(long)]
    src_mac: String,

    /// destination MAC address
    #[clap(long)]
    dest_mac: String,

    /// source IP address
    #[clap(long)]
    src_ip: String,

    /// source port
    #[clap(long)]
    src_port: u16,

    /// destination IP address
    #[clap(long)]
    dest_ip: String,

    /// destination port
    #[clap(long)]
    dest_port: u16,

    /// A level of verbosity, and can be used multiple times
    #[clap(short, long, parse(from_occurrences))]
    verbose: i32,

    /// Transmit or Receive mode
    #[clap(subcommand)]
    mode: Mode,

    /// Number of packets to transmit or wait to receive
    #[clap(short, long)]
    n_pkts: u64,

    /// Number of seconds before the sender/receiver will timeout
    #[clap(short, long)]
    timeout_sec: Option<u64>,
}

fn main() {
    env_logger::init();
    let opts: Opts = Opts::parse();

    let umem_config = UmemConfigBuilder::new()
        .frame_count(8192)
        .comp_queue_size(4096)
        .fill_queue_size(4096)
        .build()
        .unwrap();

    let socket_config = SocketConfigBuilder::new()
        .tx_queue_size(4096)
        .rx_queue_size(4096)
        .bind_flags(BindFlags::XDP_COPY)
        .xdp_flags(XdpFlags::XDP_FLAGS_SKB_MODE)
        .build()
        .unwrap();

    let n_tx_frames = umem_config.frame_count() / 2;

    let dev_ifname = opts.dev.clone();
    let xsk = Xsk2::new(
        &dev_ifname,
        0,
        umem_config,
        socket_config,
        n_tx_frames as usize,
    )
    .expect("failed to build xsk");

    match opts.mode {
        Mode::Tx => spawn_tx(xsk, opts),
        Mode::Rx => spawn_rx(xsk, opts),
    }
}

fn spawn_tx(mut xsk: Xsk2, opts: Opts) {
    eprintln!("sending {} pkts", opts.n_pkts);

    let src_mac = parse_mac(&opts.src_mac).expect("failed to parse src mac addr");
    let dest_mac = parse_mac(&opts.dest_mac).expect("failed to parse dest mac addr");
    let filter = Filter::new(&opts.src_ip, opts.src_port, &opts.dest_ip, opts.dest_port).unwrap();

    let mut pkt: [u8; MAX_PACKET_SIZE] = [0; MAX_PACKET_SIZE];
    let mut payload: [u8; 8] = [0; 8];

    let start = Instant::now();
    let timeout = match opts.timeout_sec {
        Some(timeout) => Duration::from_secs(timeout),
        None => Duration::from_secs(DEFAULT_TIMEOUT),
    };

    for i in 0..opts.n_pkts {
        if start.elapsed() > timeout {
            break;
        }
        //thread::sleep(Duration::from_millis(1));
        let pkt_builder = PacketBuilder::ethernet2(src_mac, dest_mac)
            .ipv4(filter.src_ip, filter.dest_ip, 20)
            .udp(filter.src_port, filter.dest_port);

        LittleEndian::write_u64(&mut payload, i);
        let len_pkt = generate_pkt(&mut pkt[..], &mut payload[..], pkt_builder);

        while let Err(_) = xsk.tx.send(&pkt[..len_pkt]) {}
    }

    let tx_stats = xsk.tx.stats();
    eprintln!("tx_stats = {:?}", tx_stats);
    eprintln!("tx duration = {:?}", tx_stats.duration());
    eprintln!("tx pps = {:?}", tx_stats.pps());
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

#[derive(Debug, Clone)]
struct Filter {
    src_ip: [u8; 4],
    src_port: u16,
    dest_ip: [u8; 4],
    dest_port: u16,
}

impl Filter {
    fn new(
        src_ip: &str,
        src_port: u16,
        dest_ip: &str,
        dest_port: u16,
    ) -> Result<Self, Box<dyn Error>> {
        let src_ipv4: Ipv4Addr = src_ip.parse()?;
        let dest_ipv4: Ipv4Addr = dest_ip.parse()?;

        Ok(Self {
            src_ip: src_ipv4.octets(),
            src_port,
            dest_ip: dest_ipv4.octets(),
            dest_port,
        })
    }
}

const DEFAULT_TIMEOUT: u64 = 30;

fn spawn_rx(mut xsk: Xsk2, opts: Opts) {
    let filter = Filter::new(&opts.src_ip, opts.src_port, &opts.dest_ip, opts.dest_port).unwrap();
    let mut pkt: [u8; MAX_PACKET_SIZE] = [0; MAX_PACKET_SIZE];
    let mut matched_recvd_pkts = 0;
    let mut recvd_nums: HashSet<u64> = HashSet::with_capacity(opts.n_pkts as usize);
    let start = Instant::now();
    let timeout = match opts.timeout_sec {
        Some(timeout) => Duration::from_secs(timeout),
        None => Duration::from_secs(DEFAULT_TIMEOUT),
    };

    while matched_recvd_pkts != opts.n_pkts && start.elapsed() < timeout {
        let len_recvd = xsk.rx.recv(&mut pkt[..]);
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
    }

    let expected_recvd_nums: Vec<u64> = (0..opts.n_pkts).into_iter().collect();

    let mut n_missing = 0;
    for n in expected_recvd_nums.iter() {
        if !recvd_nums.contains(n) {
            //eprintln!("missing {}", n);
            n_missing += 1;
        }
    }
    eprintln!("missing {} packets", n_missing);

    let rx_stats = xsk.rx.stats();
    eprintln!("rx_stats = {:?}", rx_stats);
    eprintln!("rx duration = {:?}", rx_stats.duration());
    eprintln!("rx pps = {:?}", rx_stats.pps());
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

fn parse_mac(mac: &str) -> Result<[u8; 6], Box<dyn Error>> {
    let mut mac_bytes: [u8; 6] = [0; 6];
    let parts: Vec<&str> = mac.split(':').into_iter().collect();
    if parts.len() != 6 {
        Err("wrong len".into())
    } else {
        for (i, part) in parts.iter().enumerate() {
            let mac_byte = u8::from_str_radix(part, 16)?;
            mac_bytes[i] = mac_byte;
        }
        Ok(mac_bytes)
    }
}

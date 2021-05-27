use std::error::Error;
use std::net::Ipv4Addr;
use std::thread;
use std::time::{Duration, Instant};

use byteorder::{ByteOrder, LittleEndian};
use clap::Clap;
use etherparse::{
    InternetSlice, PacketBuilder, PacketBuilderStep, SlicedPacket, TransportSlice, UdpHeader,
};

use xdpsock::{
    iface::InterfaceStats,
    socket::{BindFlags, SocketConfig, SocketConfigBuilder, XdpFlags},
    umem::{UmemConfig, UmemConfigBuilder},
    xsk::{Xsk2, MAX_PACKET_SIZE},
};

#[derive(Clap, Debug, Clone)]
enum Mode {
    Tx,
    Rx,
}

#[derive(Clap, Debug, Clone)]
enum BindOption {
    Copy,
    ZeroCopy,
}

#[derive(Clap, Debug, Clone)]
enum SocketOption {
    Drv,
    Skb,
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

    /// Which TX/RX queue to use
    #[clap(short, long, default_value = "0")]
    queues: Vec<u32>,

    /// Copy or ZeroCopy
    #[clap(long, arg_enum, default_value = "copy")]
    bind_option: BindOption,

    /// Copy or ZeroCopy
    #[clap(long, arg_enum, default_value = "skb")]
    socket_option: SocketOption,

    /// Number of seconds before the sender/receiver will timeout
    #[clap(short, long)]
    timeout_sec: Option<u64>,

    /// Number of frames
    #[clap(short, long, default_value = "8192")]
    xdp_queue_size: u32,

    /// Tx batch size
    #[clap(short, long, default_value = "1")]
    batch_size: usize,
}

#[derive(Debug, Clone)]
struct Filter {
    src_mac: [u8; 6],
    dest_mac: [u8; 6],
    src_ip: [u8; 4],
    src_port: u16,
    dest_ip: [u8; 4],
    dest_port: u16,
}

impl Filter {
    fn from_opts(opts: &Opts) -> Result<Self, Box<dyn Error>> {
        let src_mac = parse_mac(&opts.src_mac)?;
        let dest_mac = parse_mac(&opts.dest_mac)?;
        let src_ipv4: Ipv4Addr = opts.src_ip.parse()?;
        let dest_ipv4: Ipv4Addr = opts.dest_ip.parse()?;

        Ok(Self {
            src_mac,
            dest_mac,
            src_ip: src_ipv4.octets(),
            src_port: opts.src_port,
            dest_ip: dest_ipv4.octets(),
            dest_port: opts.dest_port,
        })
    }
}

fn main() {
    env_logger::init();
    let opts: Opts = Opts::parse();
    eprintln!("{:?}", opts);
    spawn_threads(opts);
}

/// Build umem and socket configs based on opts
fn build_umem_socket_config(opts: &Opts) -> (UmemConfig, SocketConfig) {
    let n = opts.xdp_queue_size;

    let umem_config = UmemConfigBuilder::new()
        .frame_count(n)
        .comp_queue_size(n / 2)
        .fill_queue_size(n / 2)
        .build()
        .unwrap();

    let mut socket_config_init = SocketConfigBuilder::new()
        .tx_queue_size(n / 2)
        .rx_queue_size(n / 2)
        .clone();

    let socket_config_bind = match opts.bind_option {
        BindOption::ZeroCopy => socket_config_init.bind_flags(BindFlags::XDP_ZEROCOPY),
        BindOption::Copy => socket_config_init.bind_flags(BindFlags::XDP_COPY),
    };

    let socket_config_sk = match opts.socket_option {
        SocketOption::Drv => socket_config_bind.xdp_flags(XdpFlags::XDP_FLAGS_DRV_MODE),
        SocketOption::Skb => socket_config_bind.xdp_flags(XdpFlags::XDP_FLAGS_SKB_MODE),
    };

    let socket_config = socket_config_sk.build().unwrap();
    (umem_config, socket_config)
}

/// Build an Xsk for a given queue
fn build_xsk(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    ifname: &str,
    queue: u32,
    tx_batch_size: usize,
) -> Xsk2 {
    let n_tx_frames = umem_config.frame_count() / 2;
    let xsk = Xsk2::new(
        ifname,
        queue,
        umem_config,
        socket_config,
        n_tx_frames as usize,
        tx_batch_size,
    )
    .expect("failed to build xsk");
    xsk
}

fn spawn_threads(opts: Opts) {
    let mut handles = vec![];
    let n_queues = opts.queues.len();
    let queues = opts.queues.clone();

    let iface_stats_start = InterfaceStats::new(&opts.dev);
    eprintln!("{:#?}", iface_stats_start);

    let start = Instant::now();
    for (i, queue) in queues.into_iter().enumerate() {
        let (umem_config, socket_config) = build_umem_socket_config(&opts);
        let ifname = opts.dev.clone();
        let opts = opts.clone();

        let handle = thread::spawn(move || {
            let xsk = build_xsk(umem_config, socket_config, &ifname, queue, opts.batch_size);
            let filter = Filter::from_opts(&opts).expect("failed to build packet filter");

            let pkts = split_pkts(opts.n_pkts, i as u64, n_queues as u64);
            match opts.mode {
                Mode::Tx => tx_pkts(xsk, filter, pkts),
                Mode::Rx => rx_pkts(xsk, filter, opts.n_pkts, opts.timeout_sec),
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("failed to join tx/rx handle");
    }

    let end = start.elapsed();
    let duration_secs = end.as_secs_f64();
    let pps = opts.n_pkts as f64 / duration_secs;

    match opts.mode {
        Mode::Tx => eprintln!("XDP Tx Stats"),
        Mode::Rx => eprintln!("XDP Rx Stats"),
    }

    eprintln!(
        "{} pkts in {} seconds, {} pps",
        opts.n_pkts, duration_secs, pps
    );

    let iface_stats_end = InterfaceStats::new(&opts.dev);
    let iface_stats = iface_stats_end - iface_stats_start;
    eprintln!("NIC Stats");
    match opts.mode {
        Mode::Tx => eprintln!("{:#?}", iface_stats.tx),
        Mode::Rx => eprintln!("{:#?}", iface_stats.rx),
    }
}

fn split_pkts(n_pkts: u64, queue_index: u64, n_queues: u64) -> impl Iterator<Item = u64> {
    (0..n_pkts)
        .into_iter()
        .filter(move |i| i % n_queues == queue_index)
}

fn tx_pkts(mut xsk: Xsk2, filter: Filter, pkts: impl Iterator<Item = u64>) {
    let mut prefill_pkt: [u8; MAX_PACKET_SIZE] = [0; MAX_PACKET_SIZE];
    let mut payload: [u8; 8] = [0; 8];

    let pkt_builder = PacketBuilder::ethernet2(filter.src_mac, filter.dest_mac)
        .ipv4(filter.src_ip, filter.dest_ip, 20)
        .udp(filter.src_port, filter.dest_port);
    let len_pkt = generate_pkt(&mut prefill_pkt[..], &mut payload[..], pkt_builder);

    xsk.tx.prefill(&prefill_pkt[..len_pkt]);

    for i in pkts {
        while let Err(_) = xsk.tx.send_apply(|pkt| {
            let mut payload = &mut pkt[len_pkt - 8..len_pkt];
            LittleEndian::write_u64(&mut payload, i);
            drop(payload);
            len_pkt + 8
        }) {}
    }

    xsk.tx.drain();
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

const DEFAULT_TIMEOUT: u64 = 30;

fn rx_pkts(mut xsk: Xsk2, filter: Filter, n_pkts: u64, timeout_sec: Option<u64>) {
    let mut matched_recvd_pkts = 0;
    let mut recvd_nums = vec![false; n_pkts as usize];
    let start = Instant::now();

    let timeout = match timeout_sec {
        Some(timeout) => Duration::from_secs(timeout),
        None => Duration::from_secs(DEFAULT_TIMEOUT),
    };

    let mut i = 0;

    while matched_recvd_pkts != n_pkts {
        i += 1;
        if i % 65_536 == 0 {
            if start.elapsed() > timeout {
                break;
            }
        }

        xsk.rx
            .recv_apply(|pkt| match SlicedPacket::from_ethernet(&pkt) {
                Ok(pkt) => {
                    if filter_pkt(&pkt, &filter) {
                        let n = LittleEndian::read_u64(&pkt.payload[..8]);
                        recvd_nums[n as usize] = true;
                        matched_recvd_pkts += 1;
                    }
                }
                Err(e) => log::warn!("failed to parse packet {:?}", e),
            });
        /*
        .recv_apply(|_| {});
        */
    }

    let mut n_missing = 0;
    for (i, recvd) in recvd_nums.iter().enumerate() {
        if !recvd {
            log::debug!("missing {}", i);
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

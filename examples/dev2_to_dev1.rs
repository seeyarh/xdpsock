use std::collections::HashSet;
use std::error::Error;
use std::io::Cursor;
use std::net::Ipv4Addr;
use std::thread;
use std::time::{Duration, Instant};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use clap::Clap;
use etherparse::{
    IpHeader, PacketBuilder, PacketBuilderStep, ReadError, TransportHeader, UdpHeader,
};

use xsk_rs::{
    socket::{SocketConfig, SocketConfigBuilder},
    umem::{UmemConfig, UmemConfigBuilder},
    xsk::{ParsedPacket, Xsk2},
};

#[derive(Clap, Debug)]
enum Mode {
    Tx,
    Rx,
}

/// This doc string acts as a help message when the user runs '--help'
/// as do all doc strings on fields
#[derive(Clap)]
#[clap(version = "1.0", author = "Collins Huff")]
struct Opts {
    /// interface name
    #[clap(short, long)]
    dev: String,
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
        .build()
        .unwrap();

    let n_tx_frames = umem_config.frame_count() / 2;

    let dev_ifname = opts.dev.clone();
    let mut xsk = Xsk2::new(
        &dev_ifname,
        0,
        umem_config,
        socket_config,
        n_tx_frames as usize,
    );

    match opts.mode {
        Mode::Tx => spawn_tx(xsk, opts),
        Mode::Rx => spawn_rx(xsk, opts),
    }
}

fn spawn_tx(mut xsk: Xsk2, opts: Opts) {
    let filter = Filter::new(&opts.src_ip, opts.src_port, &opts.dest_ip, opts.dest_port).unwrap();

    eprintln!("sending {} pkts", opts.n_pkts);
    let tx_send = xsk.tx_sender().unwrap();
    let send_handle = thread::spawn(move || {
        let start = Instant::now();
        for i in 0..opts.n_pkts {
            let pkt_builder = PacketBuilder::ethernet2([0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0])
                .ipv4(filter.src_ip, filter.dest_ip, 20)
                .udp(filter.src_port, filter.dest_port);
            let pkt_with_payload = generate_pkt(pkt_builder, i);
            tx_send.send(pkt_with_payload).unwrap();
        }
        let duration = start.elapsed();
        eprintln!("send time is: {:?}", duration);
    });

    send_handle.join().expect("failed to join tx handle");
    let tx_stats = xsk.shutdown_tx().expect("failed to shutdown tx");
    let rx_stats = xsk.shutdown_rx().expect("failed to shut down rx");
    eprintln!("tx_stats = {:?}", tx_stats);
    eprintln!("tx duration = {:?}", tx_stats.duration());
    eprintln!("tx pps = {:?}", tx_stats.pps());

    eprintln!("rx_stats = {:?}", rx_stats);
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

fn spawn_rx(mut xsk: Xsk2, opts: Opts) {
    let rx_recv = xsk.rx_receiver().unwrap();

    let filter = Filter::new(&opts.src_ip, opts.src_port, &opts.dest_ip, opts.dest_port).unwrap();

    let recv_handle = thread::spawn(move || {
        let mut recvd_nums: HashSet<u64> = HashSet::new();
        for pkt in rx_recv.iter() {
            let pkt = pkt.expect("failed to read pkt");
            if filter_pkt(&pkt, &filter) {
                let payload = pkt.payload.expect("no payload");
                let mut rdr = Cursor::new(&payload[0..8]);
                let n = rdr.read_u64::<LittleEndian>().unwrap();
                recvd_nums.insert(n);
            }
        }
        recvd_nums
    });

    thread::sleep(Duration::from_secs(30));
    let rx_stats = xsk.shutdown_rx().expect("failed to shut down rx");
    eprintln!("rx_stats = {:?}", rx_stats);
    eprintln!("rx duration = {:?}", rx_stats.duration());
    eprintln!("rx pps = {:?}", rx_stats.pps());

    let tx_stats = xsk.shutdown_tx().expect("failed to shut down tx");
    eprintln!("tx_stats = {:?}", tx_stats);

    let recvd_nums = recv_handle.join().expect("failed to join recv handle");

    let expected_recvd_nums: Vec<u64> = (0..opts.n_pkts).into_iter().collect();

    let mut n_missing = 0;
    for n in expected_recvd_nums.iter() {
        if !recvd_nums.contains(n) {
            //eprintln!("missing {}", n);
            n_missing += 1;
        }
    }
    eprintln!("missing {} packets", n_missing);
}

fn filter_pkt(pkt: &ParsedPacket, filter: &Filter) -> bool {
    let mut ip_match = false;
    let mut transport_match = false;
    if let Some(ref ip) = pkt.ip {
        if let IpHeader::Version4(ipv4) = ip {
            ip_match = (ipv4.source == filter.src_ip) && (ipv4.destination == filter.dest_ip);
        }
    }

    if let Some(ref transport) = pkt.transport {
        if let TransportHeader::Udp(udp) = transport {
            transport_match =
                (udp.source_port == filter.src_port) && (udp.destination_port == filter.dest_port);
        }
    }

    ip_match && transport_match
}

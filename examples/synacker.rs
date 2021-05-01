use std::error::Error;
use std::net::Ipv4Addr;
use std::thread;
use std::time::{Duration, Instant};

use clap::Clap;
use crossbeam_channel::{bounded, select, tick, Receiver};
use etherparse::{
    Ethernet2HeaderSlice, InternetSlice, IpHeader, LinkSlice, PacketBuilder, PacketBuilderStep,
    PacketHeaders, SlicedPacket, TransportHeader, TransportSlice, UdpHeader,
};

use xdpsock::{
    socket::{BindFlags, SocketConfig, SocketConfigBuilder, XdpFlags},
    umem::{UmemConfig, UmemConfigBuilder},
    xsk::{Xsk2, MAX_PACKET_SIZE},
};

/// Respond to all packets that match a certain filter with a synack
#[derive(Debug, Clone, Clap)]
#[clap(version = "1.0", author = "Collins Huff")]
struct Opts {
    /// interface name
    #[clap(short, long)]
    dev: String,

    /// source IP address
    #[clap(long)]
    src_ip: Option<String>,

    /// source port
    #[clap(long)]
    src_port: Option<u16>,

    /// destination IP address
    #[clap(long)]
    dest_ip: Option<String>,

    /// destination port
    #[clap(long)]
    dest_port: Option<u16>,

    /// A level of verbosity, and can be used multiple times
    #[clap(short, long, parse(from_occurrences))]
    verbose: i32,
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
    let mut xsk = Xsk2::new(
        &dev_ifname,
        0,
        umem_config,
        socket_config,
        n_tx_frames as usize,
    );

    spawn_rx(xsk, opts);
}

#[derive(Debug, Clone)]
struct Filter {
    src_ip: Option<[u8; 4]>,
    src_port: Option<u16>,
    dest_ip: Option<[u8; 4]>,
    dest_port: Option<u16>,
}

impl Filter {
    fn new(
        src_ip: &Option<String>,
        src_port: Option<u16>,
        dest_ip: &Option<String>,
        dest_port: Option<u16>,
    ) -> Result<Self, Box<dyn Error>> {
        let src_ipv4 = match src_ip {
            Some(src_ip) => {
                let src_ip: Ipv4Addr = src_ip.parse()?;
                Some(src_ip.octets())
            }
            None => None,
        };

        let dest_ipv4 = match dest_ip {
            Some(dest_ip) => {
                let dest_ip: Ipv4Addr = dest_ip.parse()?;
                Some(dest_ip.octets())
            }
            None => None,
        };

        Ok(Self {
            src_ip: src_ipv4,
            src_port,
            dest_ip: dest_ipv4,
            dest_port,
        })
    }

    fn filter(&self, parsed_pkt: &SlicedPacket) -> bool {
        log::debug!(
            "synacker: filtering pkt {:?} with filter {:?}",
            parsed_pkt,
            self
        );
        let mut ip_match = true;
        let mut transport_match = true;
        if let Some(ref ip) = parsed_pkt.ip {
            if let InternetSlice::Ipv4(ipv4) = ip {
                if let Some(self_src_ip) = self.src_ip {
                    ip_match = ipv4.source() == self_src_ip;
                }
                if let Some(self_dest_ip) = self.dest_ip {
                    ip_match = ip_match && (ipv4.destination() == self_dest_ip);
                }
            }
        }

        if let Some(ref transport) = parsed_pkt.transport {
            if let TransportSlice::Tcp(tcp) = transport {
                if let Some(self_src_port) = self.src_port {
                    transport_match = tcp.source_port() == self_src_port
                }
                if let Some(self_dest_port) = self.dest_port {
                    transport_match = tcp.destination_port() == self_dest_port
                }
            }
        }

        ip_match && transport_match
    }
}

fn spawn_rx(mut xsk: Xsk2, opts: Opts) {
    let rx_recv = xsk.rx_receiver().unwrap();

    let filter = Filter::new(&opts.src_ip, opts.src_port, &opts.dest_ip, opts.dest_port).unwrap();

    let ctrl_c_events = ctrl_channel().expect("failed to get ctrl c channel");

    loop {
        select! {
            recv(rx_recv) -> recvd =>  {
                log::debug!("synacker: received packet");
                let (pkt, len) = recvd.expect("failed to receive pkt");
                match SlicedPacket::from_ethernet(&pkt[..len]) {
                    Ok(pkt) => {
                        if filter.filter(&pkt) {
                            log::debug!("synacker: found match {:?}", pkt);

                            if let Some(synack) = generate_synack(&pkt) {
                                log::debug!("synacker: sending synack {:?}", synack);
                                xsk.send(&synack);
                            }
                        }
                    }
                    Err(e) => log::warn!("failed to parse packet {:?}", e),
                }
            }
            recv(ctrl_c_events) -> _ => {
                break;
            }
        }
    }

    thread::sleep(Duration::from_secs(30));
    let rx_stats = xsk.shutdown_rx().expect("failed to shut down rx");
    eprintln!("rx_stats = {:?}", rx_stats);
    eprintln!("rx duration = {:?}", rx_stats.duration());
    eprintln!("rx pps = {:?}", rx_stats.pps());

    let tx_stats = xsk.shutdown_tx().expect("failed to shut down tx");
    eprintln!("tx_stats = {:?}", tx_stats);
}

fn ctrl_channel() -> Result<Receiver<()>, ctrlc::Error> {
    let (sender, receiver) = bounded(100);
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })?;

    Ok(receiver)
}

fn generate_synack(recvd: &SlicedPacket) -> Option<Vec<u8>> {
    let link = match recvd.link.as_ref()? {
        LinkSlice::Ethernet2(link) => link,
    };
    let ipv4 = match recvd.ip.as_ref()? {
        InternetSlice::Ipv4(ipv4) => ipv4,
        InternetSlice::Ipv6(_, _) => return None,
    };
    let tcp = match recvd.transport.as_ref()? {
        TransportSlice::Tcp(tcp) => tcp,
        TransportSlice::Udp(_) => return None,
    };

    let src_mac = link.source();
    let src_mac = [
        src_mac[0], src_mac[1], src_mac[2], src_mac[3], src_mac[4], src_mac[5],
    ];
    let dest_mac = link.destination();
    let dest_mac = [
        dest_mac[0],
        dest_mac[1],
        dest_mac[2],
        dest_mac[3],
        dest_mac[4],
        dest_mac[5],
    ];

    let src_ip = ipv4.source();
    let src_ip = [src_ip[0], src_ip[1], src_ip[2], src_ip[3]];
    let dest_ip = ipv4.destination();
    let dest_ip = [dest_ip[0], dest_ip[1], dest_ip[2], dest_ip[3]];

    let pkt_builder = PacketBuilder::ethernet2(dest_mac, src_mac)
        .ipv4(dest_ip, src_ip, 20)
        .tcp(
            tcp.destination_port(),
            tcp.source_port(),
            tcp.sequence_number(),
            tcp.window_size(),
        )
        .syn()
        .ack(tcp.sequence_number() + 1);

    let payload = [];
    let mut result = Vec::<u8>::with_capacity(pkt_builder.size(payload.len()));

    //serialize
    pkt_builder
        .write(&mut result, &payload)
        .expect("failed to build packet");
    Some(result)
}

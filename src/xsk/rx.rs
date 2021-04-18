//! Receive end of AF_XDP socket

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::{socket::*, umem::*};

use crossbeam_channel::Sender;
use etherparse::{
    Ethernet2Header, IpHeader, PacketHeaders, ReadError, TransportHeader, VlanHeader,
};

pub struct RxStats {}
pub struct RxConfig {}

const MAX_PAYLOAD_SIZE: usize = 1500;

#[derive(Debug)]
pub struct ParsedPacket {
    pub link: Option<Ethernet2Header>,
    pub vlan: Option<VlanHeader>,
    pub ip: Option<IpHeader>,
    pub transport: Option<TransportHeader>,
    pub payload: Option<[u8; MAX_PAYLOAD_SIZE]>,
}

impl ParsedPacket {
    fn from_headers_with_payload(headers: PacketHeaders) -> Self {
        let mut payload: [u8; MAX_PAYLOAD_SIZE] = [0; MAX_PAYLOAD_SIZE];
        let l = std::cmp::min(MAX_PAYLOAD_SIZE, headers.payload.len());
        let payload_slice = &mut payload[..l];
        payload_slice.copy_from_slice(&headers.payload[..l]);

        Self {
            link: headers.link,
            vlan: headers.vlan,
            ip: headers.ip,
            transport: headers.transport,
            payload: Some(payload),
        }
    }

    fn from_headers_without_payload(headers: PacketHeaders) -> Self {
        Self {
            link: headers.link,
            vlan: headers.vlan,
            ip: headers.ip,
            transport: headers.transport,
            payload: None,
        }
    }
}

// TODO: recvd packets when dropped should mark the frame desc as free
#[derive(Debug)]
pub struct XskRx<'a> {
    pub fill_q: FillQueue<'a>,
    pub rx_q: RxQueue<'a>,
    pub rx_frames: Vec<Frame<'a>>,
    pub pkts_recvd: Sender<Result<ParsedPacket, ReadError>>,
    pub outstanding_rx_frames: u64,
    pub rx_cursor: usize,
    pub poll_ms_timeout: i32,
    pub shutdown: Arc<AtomicBool>,
    pub include_payload: bool,
}

impl<'a> XskRx<'a> {
    pub fn stats(&self) -> RxStats {
        RxStats {}
    }

    pub fn start_recv(&mut self) {
        let frames_filled = unsafe { self.fill_q.produce(&mut self.rx_frames[..]) };
        log::debug!("rx: init frames added to fill_q: {}", frames_filled);

        self.recv_loop();
    }

    fn recv_loop(&mut self) {
        while !self.shutdown.load(Ordering::Relaxed) {
            let n_frames_recv = self
                .rx_q
                .poll_and_consume(&mut self.rx_frames[..], self.poll_ms_timeout)
                .unwrap();

            if n_frames_recv == 0 {
                // No frames consumed, wake up fill queue if required
                log::debug!("rx: rx_q.poll_and_consume() consumed 0 frames");
                if self.fill_q.needs_wakeup() {
                    log::debug!("waking up fill_q");
                    self.fill_q
                        .wakeup(self.rx_q.fd(), self.poll_ms_timeout)
                        .unwrap();
                }
            } else {
                log::debug!(
                    "rx: rx_q.poll_and_consume() consumed {} frames",
                    &n_frames_recv
                );

                for frame in self.rx_frames.iter().take(n_frames_recv) {
                    let data = unsafe {
                        frame
                            .read_from_umem_checked(frame.len())
                            .expect("rx: failed to read from umem")
                    };

                    match PacketHeaders::from_ethernet_slice(data) {
                        Err(e) => {
                            log::debug!("rx: failed to parse pkt {:?}", data);
                            match self.pkts_recvd.send(Err(e)) {
                                Ok(_) => {}
                                Err(e) => {
                                    log::error!(
                                        "failed to put parse error on rx channel {:?}",
                                        e.into_inner()
                                    );
                                    return;
                                }
                            }
                        }
                        Ok(packet) => {
                            let parsed = if self.include_payload {
                                ParsedPacket::from_headers_with_payload(packet)
                            } else {
                                ParsedPacket::from_headers_without_payload(packet)
                            };
                            match self.pkts_recvd.send(Ok(parsed)) {
                                Ok(_) => {}
                                Err(e) => {
                                    log::error!(
                                        "failed to put parsed pkt on rx channel: {:?}",
                                        e.into_inner()
                                    );
                                    return;
                                }
                            }
                        }
                    }
                }

                // Add frames back to fill queue
                while unsafe {
                    self.fill_q
                        .produce_and_wakeup(
                            &mut self.rx_frames[..n_frames_recv],
                            self.rx_q.fd(),
                            self.poll_ms_timeout,
                        )
                        .unwrap()
                } != n_frames_recv
                {
                    // Loop until frames added to the fill ring.
                    log::debug!("rx: fill_q.produce_and_wakeup() failed to allocate");
                }
                log::debug!(
                    "rx: fill_q.produce_and_wakeup() submitted {} frames",
                    n_frames_recv
                );
            }
        }
    }
}

//! Receive end of AF_XDP socket

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;

use crate::xsk::xsk::MAX_PACKET_SIZE;
use crate::{socket::*, umem::*};

#[derive(Debug)]
pub struct RxStats {
    pub pkts_rx: u64,
    pub pkts_rx_parse_fail: u64,
    pub start_time: Instant,
    pub end_time: Instant,
}

impl RxStats {
    pub fn new() -> Self {
        Self {
            pkts_rx: 0,
            pkts_rx_parse_fail: 0,
            start_time: Instant::now(),
            end_time: Instant::now(),
        }
    }
    pub fn duration(&self) -> Duration {
        self.end_time.duration_since(self.start_time)
    }

    pub fn pps(&self) -> f64 {
        self.pkts_rx as f64 / self.duration().as_secs_f64()
    }
}

#[derive(Debug)]
pub struct RxConfig {}

// TODO: recvd packets when dropped should mark the frame desc as free
#[derive(Debug)]
pub struct XskRx<'a> {
    pub fill_q: FillQueue<'a>,
    pub rx_q: RxQueue<'a>,
    pub rx_frames: Vec<Frame<'a>>,
    pub pkts_recvd: Sender<([u8; MAX_PACKET_SIZE], usize)>,
    pub outstanding_rx_frames: u64,
    pub rx_cursor: usize,
    pub poll_ms_timeout: i32,
    pub shutdown: Arc<AtomicBool>,
    pub include_payload: bool,
    pub stats: RxStats,
}

impl<'a> XskRx<'a> {
    pub fn start_recv(&mut self) {
        let frames_filled = unsafe { self.fill_q.produce(&mut self.rx_frames[..]) };
        log::debug!("rx: init frames added to fill_q: {}", frames_filled);

        self.recv_loop();
        self.stats.end_time = Instant::now();
    }

    // TODO: Do we need an RX cursor? Will the addresses received in poll and consume always be in
    // the correct order?
    fn recv_loop(&mut self) {
        while !self.shutdown.load(Ordering::Relaxed) {
            // check for rx packets
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
                // frames consumed
                log::debug!(
                    "rx: rx_q.poll_and_consume() consumed {} frames",
                    &n_frames_recv
                );

                // Iterate over packets received, parse and send on channel
                for frame in self.rx_frames.iter().take(n_frames_recv) {
                    let data = unsafe {
                        frame
                            .read_from_umem_checked(frame.len())
                            .expect("rx: failed to read from umem")
                    };

                    let mut packet: [u8; MAX_PACKET_SIZE] = [0; MAX_PACKET_SIZE];
                    let l = std::cmp::min(MAX_PACKET_SIZE, data.len());
                    let packet_slice = &mut packet[..l];
                    packet_slice.copy_from_slice(&data[..l]);

                    match self.pkts_recvd.send((packet, data.len())) {
                        Ok(_) => {
                            self.stats.pkts_rx += 1;
                        }
                        Err(e) => {
                            log::error!(
                                "failed to put parsed packet on rx channel {:?}",
                                e.into_inner()
                            );
                            return;
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

        self.stats.end_time = Instant::now();
    }
}

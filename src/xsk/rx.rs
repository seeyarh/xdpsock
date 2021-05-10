//! Receive end of AF_XDP socket

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{socket::*, umem::*};

#[derive(Debug, Clone)]
pub struct RxStats {
    pub pkts_rx: u64,
    pub pkts_rx_delivered: u64,
    pub start_time: Instant,
    pub end_time: Instant,
}

impl RxStats {
    pub fn new() -> Self {
        Self {
            pkts_rx: 0,
            pkts_rx_delivered: 0,
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
    //_umem: Arc<Umem<'a>>,
    pub fill_q: FillQueue<'a>,
    pub rx_q: RxQueue<'a>,
    pub rx_frames: Vec<Frame<'a>>,
    pub filled_frames: Vec<(usize, usize, u32)>,
    pub rx_frame_offset: usize,
    pub n_frames_to_be_filled: u64,
    pub frame_size: u32,
    pub rx_cursor: usize,
    pub poll_ms_timeout: i32,
    pub stats: RxStats,
}

impl<'a> XskRx<'a> {
    pub fn new(
        //umem: Arc<Umem<'a>>,
        rx_q: RxQueue<'a>,
        mut fill_q: FillQueue<'a>,
        rx_frames: Vec<Frame<'a>>,
        frame_size: u32,
    ) -> Self {
        let n_rx_frames = rx_frames.len();

        let mut init_rx_frames: Vec<&Frame> = rx_frames.iter().collect();
        let rx_frame_offset = init_rx_frames[0].addr();
        log::debug!("init_rx_frames[0] = {:?}", init_rx_frames[0]);
        log::debug!(
            "init_rx_frames[-1] = {:?}",
            init_rx_frames[init_rx_frames.len() - 1]
        );

        let frames_filled = unsafe { fill_q.produce(&mut init_rx_frames[..]) };
        log::debug!("rx: init frames added to fill_q: {}", frames_filled);
        Self {
            //_umem: umem,
            rx_q,
            fill_q,
            rx_frames,
            filled_frames: vec![(0, 0, 0); n_rx_frames],
            rx_frame_offset,
            n_frames_to_be_filled: n_rx_frames as u64,
            frame_size,
            rx_cursor: 0,
            poll_ms_timeout: 0,
            stats: RxStats::new(),
        }
    }

    pub fn recv(&mut self, pkt_receiver: &mut [u8]) -> usize {
        // check for rx packets
        let n_frames_recv = self
            .rx_q
            .poll_and_consume(
                self.n_frames_to_be_filled,
                &mut self.filled_frames[..],
                self.poll_ms_timeout,
            )
            .unwrap();

        self.stats.pkts_rx += n_frames_recv as u64;

        self.n_frames_to_be_filled -= n_frames_recv as u64;

        if n_frames_recv == 0 {
            // No frames consumed, wake up fill queue if required
            //log::debug!("rx: rx_q.poll_and_consume() consumed 0 frames");
            if self.fill_q.needs_wakeup() {
                log::debug!("waking up fill_q");
                self.fill_q
                    .wakeup(self.rx_q.fd(), self.poll_ms_timeout)
                    .unwrap();
            }
        }

        // frames consumed
        log::debug!(
            "rx: rx_q.poll_and_consume() consumed {} frames",
            &n_frames_recv
        );

        self.update_rx_frames(n_frames_recv);

        let frame = &self.rx_frames[self.rx_cursor];

        match frame.status {
            FrameStatus::Filled => {
                let data = unsafe {
                    frame
                        .read_from_umem_checked(frame.len())
                        .expect("rx: failed to read from umem")
                };

                //log::debug!("rx_data: len = {} = {:?}", frame.len(), data);
                let len = frame.len();
                pkt_receiver[..len].copy_from_slice(data);

                // Add frames back to fill queue
                while unsafe {
                    self.fill_q
                        .produce_and_wakeup(&mut [frame], self.rx_q.fd(), self.poll_ms_timeout)
                        .unwrap()
                } != 1
                {
                    // Loop until frames added to the fill ring.
                    //log::debug!("rx: fill_q.produce_and_wakeup() failed to allocate");
                }

                //log::debug!("rx: fill_q.produce_and_wakeup() submitted {} frames", 1);

                self.stats.pkts_rx_delivered += 1;
                self.rx_frames[self.rx_cursor].status = FrameStatus::Free;
                self.n_frames_to_be_filled += 1;
                self.rx_cursor = (self.rx_cursor + 1) % self.rx_frames.len();
                return len;
            }
            _ => return 0,
        }
    }

    fn update_rx_frames(&mut self, n_frames_recv: usize) {
        let filled_frames = &self.filled_frames[..n_frames_recv];
        for filled_frame in filled_frames {
            //log::debug!("filled_frame = {:?}", filled_frame);
            let rx_frame_index =
                ((*filled_frame).0 as u32 - self.rx_frame_offset as u32) / self.frame_size;
            let rx_frame_addr = (*filled_frame).0;
            let rx_frame_len = (*filled_frame).1;
            let rx_frame_options = (*filled_frame).2;

            /*
            log::debug!(
                "update rx_frame, rx_frame_index = {} rx_frame_len = {}, rx_frame_options = {}",
                rx_frame_index,
                rx_frame_len,
                rx_frame_options,
            );
            */
            self.rx_frames[rx_frame_index as usize].set_addr(rx_frame_addr);
            self.rx_frames[rx_frame_index as usize].set_len(rx_frame_len);
            self.rx_frames[rx_frame_index as usize].set_options(rx_frame_options);
            self.rx_frames[rx_frame_index as usize].status = FrameStatus::Filled;
        }
    }

    pub fn stats(&self) -> RxStats {
        self.stats.clone()
    }
}

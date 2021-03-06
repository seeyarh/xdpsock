//! Transmit end of AF_XDP socket

use crate::{socket::*, umem::*};
use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct TxStats {
    pub pkts_tx: u64,
    pub pkts_tx_completed: u64,
    pub start_time: Instant,
    pub end_time: Instant,
}

impl TxStats {
    pub fn new() -> Self {
        Self {
            pkts_tx: 0,
            pkts_tx_completed: 0,
            start_time: Instant::now(),
            end_time: Instant::now(),
        }
    }

    pub fn duration(&self) -> Duration {
        self.end_time.duration_since(self.start_time)
    }

    pub fn pps(&self) -> f64 {
        self.pkts_tx as f64 / self.duration().as_secs_f64()
    }
}

#[derive(Debug)]
pub struct TxConfig {}

/// Send Error for Xsk
#[derive(Debug)]
pub enum XskSendError {
    NoFreeTxFrames,
}

impl fmt::Display for XskSendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFreeTxFrames => write!(f, "there are no free tx frames, try again later"),
        }
    }
}

impl Error for XskSendError {}

#[derive(Debug)]
pub struct XskTx {
    //_umem: Arc<Umem>,
    pub tx_q: TxQueue,
    pub comp_q: CompQueue,
    pub tx_frames: Vec<Frame>,
    pub free_frames: Vec<u64>,
    pub outstanding_tx_frames: u64,
    pub tx_poll_ms_timeout: i32,
    pub tx_cursor: usize,
    pub frame_size: u32,
    pub stats: TxStats,
    pub batch_size: usize,
    pub cur_batch_size: usize,
}

impl XskTx {
    pub fn new(
        //umem: Arc<Umem>,
        tx_q: TxQueue,
        comp_q: CompQueue,
        tx_frames: Vec<Frame>,
        frame_size: u32,
        batch_size: usize,
    ) -> Self {
        let n_tx_frames = tx_frames.len();
        Self {
            //_umem: umem,
            tx_q,
            comp_q,
            tx_frames,
            free_frames: vec![0; n_tx_frames],
            outstanding_tx_frames: 0,
            tx_poll_ms_timeout: -1,
            tx_cursor: 0,
            frame_size,
            stats: TxStats::new(),
            // TODO: Add integration tests to make sure this is working
            batch_size,
            cur_batch_size: 0,
        }
    }

    pub fn prefill(&mut self, data: &[u8]) {
        for frame in self.tx_frames.iter_mut() {
            unsafe {
                frame
                    .write_to_umem_checked(data)
                    .expect("failed to write to umem");
            }
        }
    }

    pub fn send(&mut self, data: &[u8]) -> Result<(), XskSendError> {
        log::debug!("tx: tx_cursor = {}", self.tx_cursor);

        self.complete_frames();

        if !self.tx_frames[self.tx_cursor].status.is_free() {
            return Err(XskSendError::NoFreeTxFrames);
        }

        unsafe {
            self.tx_frames[self.tx_cursor]
                .write_to_umem_checked(data)
                .expect("failed to write to umem");
        }

        self.tx_cursor = (self.tx_cursor + 1) % self.tx_frames.len();
        self.cur_batch_size += 1;

        log::debug!(
            "tx: cur_batch_size = {}, batch_size = {}",
            self.cur_batch_size,
            self.batch_size
        );

        // Add consumed frames back to the tx queue
        if self.cur_batch_size == self.batch_size {
            self.put_batch_on_tx_queue();
        }

        Ok(())
    }

    pub fn send_apply<F>(&mut self, mut f: F) -> Result<(), XskSendError>
    where
        F: FnMut(&mut [u8]) -> usize,
    {
        log::debug!("tx: tx_cursor = {}", self.tx_cursor);

        self.complete_frames();

        if !self.tx_frames[self.tx_cursor].status.is_free() {
            return Err(XskSendError::NoFreeTxFrames);
        }

        let data =
            unsafe { self.tx_frames[self.tx_cursor].umem_region_mut(self.frame_size as usize) };
        let len = f(data);
        self.tx_frames[self.tx_cursor].set_len(len);

        self.tx_cursor = (self.tx_cursor + 1) % self.tx_frames.len();
        self.cur_batch_size += 1;

        log::debug!(
            "tx: cur_batch_size = {}, batch_size = {}",
            self.cur_batch_size,
            self.batch_size
        );

        // Add consumed frames back to the tx queue
        if self.cur_batch_size == self.batch_size {
            self.put_batch_on_tx_queue();
        }

        Ok(())
    }

    fn put_batch_on_tx_queue(&mut self) {
        log::debug!(
            "tx: putting batch on queue: batch_size = {}, tx_cursor = {}",
            self.batch_size,
            self.tx_cursor
        );
        if self.cur_batch_size == 0 {
            return;
        }

        let (start, end) = if self.tx_cursor == 0 {
            let start = self.tx_frames.len() - self.cur_batch_size;
            let end = self.tx_frames.len();
            (start, end)
        } else {
            let start = self.tx_cursor - self.cur_batch_size;
            let end = self.tx_cursor;
            (start, end)
        };
        log::debug!("tx: adding tx_frames[{}..{}] to tx queue", start, end);

        for frame in self.tx_frames[start..end].iter_mut() {
            frame.status = FrameStatus::OnTxQueue;
        }

        while unsafe {
            self.tx_q
                .produce_and_wakeup(&self.tx_frames[start..end])
                .expect("failed to add frames to tx queue")
        } != self.cur_batch_size
        {
            // Loop until frames added to the tx ring.
            log::debug!(
                "tx_q.produce_and_wakeup() failed to allocate {} frame",
                self.cur_batch_size
            );
        }
        log::debug!("tx_q.produce_and_wakeup() submitted {} frames", 1);

        self.stats.pkts_tx += self.cur_batch_size as u64;
        self.outstanding_tx_frames += self.cur_batch_size as u64;
        self.cur_batch_size = 0;
    }

    /// Read frames from completion queue
    fn complete_frames(&mut self) -> u64 {
        log::debug!("tx: reading from completion queue");
        let n_free_frames = self
            .comp_q
            .consume(self.outstanding_tx_frames, &mut self.free_frames);
        self.outstanding_tx_frames -= n_free_frames;

        self.stats.pkts_tx_completed += n_free_frames;

        /*
        if n_free_frames == 0 {
            log::debug!("comp_q.consume() consumed 0 frames");
            if self.tx_q.needs_wakeup() {
                log::debug!("tx: waking up tx_q");
                self.tx_q.wakeup().expect("failed to wake up tx queue");
                log::debug!("tx: woke up tx_q");
            }
        }
        */
        log::debug!("tx: comp_q.consume() consumed {} frames", n_free_frames);

        self.update_tx_frames(n_free_frames as usize);
        n_free_frames
    }

    /// Wait until all outstanding tx packets have come back on the comp queue
    pub fn drain(&mut self) {
        loop {
            self.complete_frames();
            self.put_batch_on_tx_queue();
            if self.outstanding_tx_frames == 0 {
                break;
            }
        }
    }

    fn update_tx_frames(&mut self, n_free_frames: usize) {
        let free_frames = &self.free_frames[..n_free_frames];
        for free_frame in free_frames {
            let tx_frame_index = *free_frame as u32 / self.frame_size;
            log::debug!(
                "update_tx_frame, tx_frame_index = {}, free_frame = {}",
                tx_frame_index,
                free_frame
            );
            self.tx_frames[tx_frame_index as usize].status = FrameStatus::Free;
        }
    }

    pub fn stats(&self) -> TxStats {
        let mut stats = self.stats.clone();
        stats.end_time = Instant::now();
        stats
    }
}

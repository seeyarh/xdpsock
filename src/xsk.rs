//! AF_XDP socket
use crate::{socket::*, umem::*};

use log::debug;
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{thread, thread::spawn, thread::JoinHandle};

use crossbeam_channel::{bounded, Receiver, Sender};

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

pub struct SenderStats {}
pub struct ReceiverStats {}

/// AF_XDP socket
pub struct Xsk<'a> {
    pub if_name: &'a str,
    pub fill_q: FillQueue<'a>,
    pub comp_q: CompQueue<'a>,
    pub tx_q: TxQueue<'a>,
    pub rx_q: RxQueue<'a>,
    pub tx_frames: Vec<Frame<'a>>,
    pub rx_frames: Vec<Frame<'a>>,
    pub umem: Umem<'a>,
    pub umem_config: UmemConfig,
    pub socket_config: SocketConfig,
    outstanding_tx_frames: u64,
    outstanding_rx_frames: u64,
    tx_poll_ms_timeout: i32,
    tx_cursor: usize,
    rx_cursor: usize,
}

impl<'a> Xsk<'a> {
    pub fn new(
        if_name: &'a str,
        queue_id: u32,
        umem_config: UmemConfig,
        socket_config: SocketConfig,
        n_tx_frames: usize,
    ) -> Self {
        let (mut umem, fill_q, comp_q, frames) = Umem::builder(umem_config.clone())
            .create_mmap()
            .expect("failed to create mmap area")
            .create_umem()
            .expect("failed to create umem");

        let (tx_q, rx_q) = Socket::new(socket_config.clone(), &mut umem, if_name, queue_id)
            .expect("failed to build socket");

        let tx_frames = frames[..n_tx_frames].into();
        let rx_frames = frames[n_tx_frames..].into();

        Self {
            if_name,
            fill_q,
            comp_q,
            tx_q,
            rx_q,
            tx_frames,
            rx_frames,
            umem,
            umem_config,
            socket_config,
            outstanding_tx_frames: 0,
            outstanding_rx_frames: 0,
            tx_poll_ms_timeout: 10,
            tx_cursor: 0,
            rx_cursor: 0,
        }
    }
}

/// AF_XDP socket
pub struct Xsk2<'a> {
    pub ifname: &'a str,
    pub umem: Umem<'a>,
    pub umem_config: UmemConfig,
    pub socket_config: SocketConfig,
    tx_handle: Option<JoinHandle<SenderStats>>,
    tx_channel: Option<Sender<Vec<u8>>>,
    rx_handle: Option<JoinHandle<ReceiverStats>>,
    rx_channel: Option<Receiver<Vec<u8>>>,
    shutdown: Arc<AtomicBool>,
}

impl<'a> Xsk2<'a> {
    pub fn new(
        if_name: &'a str,
        queue_id: u32,
        umem_config: UmemConfig,
        socket_config: SocketConfig,
        n_tx_frames: usize,
    ) -> Self {
        let (mut umem, fill_q, comp_q, frames) = Umem::builder(umem_config.clone())
            .create_mmap()
            .expect("failed to create mmap area")
            .create_umem()
            .expect("failed to create umem");

        let (tx_q, rx_q) = Socket::new(socket_config.clone(), &mut umem, if_name, queue_id)
            .expect("failed to build socket");

        let tx_frames = frames[..n_tx_frames].into();
        let rx_frames = frames[n_tx_frames..].into();

        let tx_channel_capacity = 1;
        let rx_channel_capacity = 10_000;

        let (tx_pkt_send, tx_pkt_recv) = bounded(tx_channel_capacity);
        let (rx_pkt_send, rx_pkt_recv) = bounded(rx_channel_capacity);

        let shutdown = Arc::new(AtomicBool::new(false));

        let mut xsk_tx = XskTx {
            tx_q,
            comp_q,
            tx_frames,
            pkts_to_send: tx_pkt_recv,
            outstanding_tx_frames: 0,
            tx_poll_ms_timeout: 5,
            tx_cursor: 0,
            frame_size: umem_config.frame_size(),
            shutdown: shutdown.clone(),
        };
        debug!("xsk_tx len frames = {}", xsk_tx.tx_frames.len());

        let mut xsk_rx = XskRx {
            rx_q,
            fill_q,
            rx_frames,
            pkts_recvd: rx_pkt_send,
            outstanding_rx_frames: 0,
            rx_cursor: 0,
            poll_ms_timeout: 5,
            shutdown: shutdown.clone(),
        };

        let tx_handle = spawn(move || {
            xsk_tx.send_loop();
            xsk_tx.stats()
        });

        let rx_handle = spawn(move || {
            xsk_rx.start_recv();
            xsk_rx.recv_loop();
            xsk_rx.stats()
        });

        Self {
            ifname: if_name,
            umem,
            umem_config,
            socket_config,
            tx_handle: Some(tx_handle),
            tx_channel: Some(tx_pkt_send),
            rx_handle: Some(rx_handle),
            rx_channel: Some(rx_pkt_recv),
            shutdown,
        }
    }

    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(tx_channel) = self.tx_channel.take() {
            drop(tx_channel);
        }
        if let Some(rx_channel) = self.rx_channel.take() {
            drop(rx_channel);
        }
        if let Some(tx_handle) = self.tx_handle.take() {
            tx_handle.join().expect("failed to join tx_handle");
        }
        if let Some(rx_handle) = self.rx_handle.take() {
            rx_handle.join().expect("failed to join rx_handle");
        }
    }

    pub fn send(&mut self, data: &[u8]) {
        if let Some(ref mut tx_channel) = self.tx_channel {
            tx_channel.send(data.into()).expect("failed to send");
        }
    }

    pub fn tx_sender(&self) -> Option<Sender<Vec<u8>>> {
        if let Some(ref tx_channel) = self.tx_channel {
            Some(tx_channel.clone())
        } else {
            None
        }
    }

    pub fn rx_receiver(&self) -> Option<Receiver<Vec<u8>>> {
        if let Some(ref rx_channel) = self.rx_channel {
            Some(rx_channel.clone())
        } else {
            None
        }
    }

    pub fn recv(&mut self) -> Option<Vec<u8>> {
        if let Some(ref rx_channel) = self.rx_channel {
            let recvd = rx_channel.recv().expect("failed to recv");
            Some(recvd)
        } else {
            None
        }
    }
}

struct XskTx<'a> {
    tx_q: TxQueue<'a>,
    comp_q: CompQueue<'a>,
    tx_frames: Vec<Frame<'a>>,
    pkts_to_send: Receiver<Vec<u8>>,
    outstanding_tx_frames: u64,
    tx_poll_ms_timeout: i32,
    tx_cursor: usize,
    frame_size: u32,
    shutdown: Arc<AtomicBool>,
}

impl<'a> XskTx<'a> {
    fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    fn stats(&self) -> SenderStats {
        SenderStats {}
    }

    fn send_loop(&mut self) {
        let pkt_iter = self.pkts_to_send.clone().into_iter();
        for data in pkt_iter {
            loop {
                log::debug!("tx: sending {:?}", data);
                log::debug!("tx: channel len = {}", self.pkts_to_send.len());
                match self.send(&data) {
                    Ok(_) => break,
                    Err(e) => match e {
                        XskSendError::NoFreeTxFrames => {
                            log::debug!("tx: No tx frames");
                            thread::sleep(Duration::from_millis(5));
                        }
                    },
                }
            }
        }
    }

    fn send(&mut self, data: &[u8]) -> Result<(), XskSendError> {
        log::debug!("tx: comp_q consume");
        let free_frames = self.comp_q.consume(self.outstanding_tx_frames);
        self.outstanding_tx_frames -= free_frames.len() as u64;
        log::debug!("tx: comp_q consume done");

        if free_frames.len() == 0 {
            log::debug!("comp_q.consume() consumed 0 frames");
            if self.tx_q.needs_wakeup() {
                log::debug!("tx: waking up tx_q");
                self.tx_q.wakeup().expect("failed to wake up tx queue");
                log::debug!("tx: woke up tx_q");
            }
        }
        log::debug!(
            "dev2.comp_q.consume() consumed {} frames",
            free_frames.len()
        );

        self.update_tx_frames(&free_frames);

        if !self.tx_frames[self.tx_cursor].status.is_free() {
            return Err(XskSendError::NoFreeTxFrames);
        }

        unsafe {
            self.tx_frames[self.tx_cursor]
                .write_to_umem_checked(data)
                .expect("failed to write to umem");
        }

        /*
        // Wait until we're ok to write
        log::debug!("tx: poll write tx q");
        while !poll_write(self.tx_q.fd(), self.tx_poll_ms_timeout).unwrap() {
            log::debug!("tx: poll_write(dev2.tx_q) returned false");
            continue;
        }
        log::debug!("tx: done polling");
        */

        // Add consumed frames back to the tx queue
        while unsafe {
            self.tx_q
                .produce_and_wakeup(&self.tx_frames[self.tx_cursor..self.tx_cursor + 1])
                .expect("failed to add frames to tx queue")
        } != 1
        {
            // Loop until frames added to the tx ring.
            log::debug!("tx_q.produce_and_wakeup() failed to allocate");
        }
        log::debug!("tx_q.produce_and_wakeup() submitted {} frames", 1);

        self.outstanding_tx_frames += 1;
        self.tx_frames[self.tx_cursor].status = FrameStatus::OnTxQueue;
        self.tx_cursor = (self.tx_cursor + 1) % self.tx_frames.len();
        Ok(())
    }

    //TODO: This is stupidly slow. maybe try maintaining a free cursor.
    fn update_tx_frames(&mut self, free_frames: &[u64]) {
        for free_frame in free_frames {
            let tx_frame_index = *free_frame as u32 / self.frame_size;
            log::debug!(
                "update tx_frame, tx_frame_index = {}, free_frame = {}",
                tx_frame_index,
                free_frame
            );
            self.tx_frames[tx_frame_index as usize].status = FrameStatus::Free;
        }
    }
}

// TODO: recvd packets when dropped should mark the frame desc as free
#[derive(Debug)]
struct XskRx<'a> {
    pub fill_q: FillQueue<'a>,
    pub rx_q: RxQueue<'a>,
    pub rx_frames: Vec<Frame<'a>>,
    pkts_recvd: Sender<Vec<u8>>,
    outstanding_rx_frames: u64,
    rx_cursor: usize,
    poll_ms_timeout: i32,
    shutdown: Arc<AtomicBool>,
}

impl<'a> XskRx<'a> {
    fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    fn recv_loop(&mut self) {
        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }
            for pkt in self.recv() {
                self.pkts_recvd
                    .send(pkt)
                    .expect("failed to put rcvd pkt on channel");
            }
        }
    }

    fn stats(&self) -> ReceiverStats {
        ReceiverStats {}
    }

    fn start_recv(&mut self) {
        let frames_filled = unsafe { self.fill_q.produce(&mut self.rx_frames[..]) };
        log::debug!("init frames added to fill_q: {}", frames_filled);
    }

    fn recv(&mut self) -> Vec<Vec<u8>> {
        let mut recv_frames = vec![];
        let n_frames_recv = self
            .rx_q
            .poll_and_consume(&mut self.rx_frames[..], self.poll_ms_timeout)
            .unwrap();

        if n_frames_recv == 0 {
            // No frames consumed, wake up fill queue if required
            log::debug!("rx_q.poll_and_consume() consumed 0 frames");
            if self.fill_q.needs_wakeup() {
                log::debug!("waking up fill_q");
                self.fill_q
                    .wakeup(self.rx_q.fd(), self.poll_ms_timeout)
                    .unwrap();
            }
        } else {
            log::debug!("rx_q.poll_and_consume() consumed {} frames", &n_frames_recv);

            for frame in self.rx_frames.iter().take(n_frames_recv) {
                let data = unsafe {
                    frame
                        .read_from_umem_checked(frame.len())
                        .expect("failed to read from umem")
                };

                recv_frames.push(data.into());
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
                log::debug!("fill_q.produce_and_wakeup() failed to allocate");
            }
            log::debug!(
                "fill_q.produce_and_wakeup() submitted {} frames",
                n_frames_recv
            );
        }
        recv_frames
    }
}

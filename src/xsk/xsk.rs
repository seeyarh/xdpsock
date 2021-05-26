//! AF_XDP socket
use super::rx::*;
use super::tx::*;
use crate::{socket::*, umem::*};

use std::error::Error;

pub const MAX_PACKET_SIZE: usize = 1500;

/// AF_XDP socket
pub struct Xsk {
    pub if_name: String,
    pub fill_q: FillQueue,
    pub comp_q: CompQueue,
    pub tx_q: TxQueue,
    pub rx_q: RxQueue,
    pub tx_frames: Vec<Frame>,
    pub rx_frames: Vec<Frame>,
    pub umem: Umem,
    pub umem_config: UmemConfig,
    pub socket_config: SocketConfig,
}

impl Xsk {
    pub fn new(
        if_name: &str,
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
            if_name: if_name.into(),
            fill_q,
            comp_q,
            tx_q,
            rx_q,
            tx_frames,
            rx_frames,
            umem,
            umem_config,
            socket_config,
        }
    }
}
/// AF_XDP socket
pub struct Xsk2 {
    pub if_name: String,
    pub tx: XskTx,
    pub rx: XskRx,
}

pub struct Xsk2Config {
    pub if_name: String,
    pub queue_id: u32,
    pub n_tx_frames: usize,
    pub tx_batch_size: usize,
    pub umem_config: UmemConfig,
    pub socket_config: SocketConfig,
}

unsafe impl Send for Xsk2 {}

impl Xsk2 {
    pub fn from_config(config: Xsk2Config) -> Result<Xsk2, Box<dyn Error>> {
        Self::new(
            &config.if_name,
            config.queue_id,
            config.umem_config,
            config.socket_config,
            config.n_tx_frames,
            config.tx_batch_size,
        )
    }
    pub fn new(
        if_name: &str,
        queue_id: u32,
        umem_config: UmemConfig,
        socket_config: SocketConfig,
        n_tx_frames: usize,
        tx_batch_size: usize,
    ) -> Result<Xsk2, Box<dyn Error>> {
        if socket_config.tx_queue_size() < tx_batch_size as u32 {
            return Err("tx batch size too large".into());
        }

        let (mut umem, fill_q, comp_q, frames) = Umem::builder(umem_config.clone())
            .create_mmap()
            .expect("failed to create mmap area")
            .create_umem()
            .expect("failed to create umem");

        log::debug!("xsk_new: creating Xsk for ifname {}, queue {}, umem_config {:?}, socket_config {:?}, n_tx_frames {}",
                    if_name, queue_id, umem_config, socket_config, n_tx_frames);
        let (tx_q, rx_q) = Socket::new(socket_config.clone(), &mut umem, if_name, queue_id)
            .expect("failed to build socket");

        let tx_frames = frames[..n_tx_frames].into();
        let rx_frames = frames[n_tx_frames..].into();

        // TODO: Fix this
        std::mem::forget(umem);

        /*
        let tx_umem = Arc::new(umem);
        let rx_umem = tx_umem.clone();

        let tx = XskTx::new(tx_umem, tx_q, comp_q, tx_frames, umem_config.frame_size());
        let rx = XskRx::new(rx_umem, rx_q, fill_q, rx_frames, umem_config.frame_size());
        */

        let tx = XskTx::new(
            tx_q,
            comp_q,
            tx_frames,
            umem_config.frame_size(),
            tx_batch_size,
        );
        let rx = XskRx::new(rx_q, fill_q, rx_frames, umem_config.frame_size());

        Ok(Xsk2 {
            if_name: if_name.into(),
            tx,
            rx,
        })
    }
}

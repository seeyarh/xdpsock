//! AF_XDP socket
use super::rx::*;
use super::tx::*;
use crate::{socket::*, umem::*};
use std::sync::Arc;

use std::error::Error;

pub const MAX_PACKET_SIZE: usize = 1500;

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
        }
    }
}
/// AF_XDP socket
pub struct Xsk2<'a> {
    pub tx: XskTx<'a>,
    pub rx: XskRx<'a>,
}

unsafe impl<'a> Send for Xsk2<'a> {}

impl<'a> Xsk2<'a> {
    pub fn new(
        if_name: &str,
        queue_id: u32,
        umem_config: UmemConfig,
        socket_config: SocketConfig,
        n_tx_frames: usize,
    ) -> Result<Xsk2<'a>, Box<dyn Error>> {
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

        let tx_umem = Arc::new(umem);
        let rx_umem = tx_umem.clone();

        let tx = XskTx::new(tx_umem, tx_q, comp_q, tx_frames, umem_config.frame_size());
        let rx = XskRx::new(rx_umem, rx_q, fill_q, rx_frames, umem_config.frame_size());

        Ok(Xsk2 { tx, rx })
    }
}

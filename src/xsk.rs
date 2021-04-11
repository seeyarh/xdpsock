//! AF_XDP socket
use crate::{socket::*, umem::*};

/// AF_XDP socket
pub struct Xsk<'a> {
    pub if_name: String,
    pub fill_q: FillQueue<'a>,
    pub comp_q: CompQueue<'a>,
    pub tx_q: TxQueue<'a>,
    pub rx_q: RxQueue<'a>,
    pub frame_descs: Vec<FrameDesc<'a>>,
    pub umem: Umem<'a>,
}

fn build_umem<'a>(
    umem_config: Option<UmemConfig>,
) -> (Umem<'a>, FillQueue<'a>, CompQueue<'a>, Vec<FrameDesc<'a>>) {
    let config = match umem_config {
        Some(cfg) => cfg,
        None => UmemConfig::default(),
    };

    Umem::builder(config)
        .create_mmap()
        .expect("failed to create mmap area")
        .create_umem()
        .expect("failed to create umem")
}

pub fn build_socket_and_umem<'a, 'umem>(
    umem_config: Option<UmemConfig>,
    socket_config: Option<SocketConfig>,
    if_name: &'a str,
    queue_id: u32,
) -> (
    (
        Umem<'umem>,
        FillQueue<'umem>,
        CompQueue<'umem>,
        Vec<FrameDesc<'umem>>,
    ),
    (TxQueue<'umem>, RxQueue<'umem>),
) {
    let socket_config = match socket_config {
        Some(cfg) => cfg,
        None => SocketConfig::default(),
    };

    let (mut umem, fill_q, comp_q, frame_descs) = build_umem(umem_config);

    let (tx_q, rx_q) =
        Socket::new(socket_config, &mut umem, if_name, queue_id).expect("failed to build socket");

    ((umem, fill_q, comp_q, frame_descs), (tx_q, rx_q))
}

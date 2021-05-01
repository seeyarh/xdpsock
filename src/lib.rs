//! # xdpsock
//!
//! A rust interface for AF_XDP sockets using libbpf.
//! ```
//! use xdpsock::{
//!    socket::{BindFlags, SocketConfig, SocketConfigBuilder, XdpFlags},
//!    umem::{UmemConfig, UmemConfigBuilder},
//!    xsk::Xsk2,
//! };
//!
//! // Configuration
//! let umem_config = UmemConfigBuilder::new()
//!    .frame_count(8192)
//!    .comp_queue_size(4096)
//!    .fill_queue_size(4096)
//!    .build()
//!    .unwrap();
//!
//! let socket_config = SocketConfigBuilder::new()
//!    .tx_queue_size(4096)
//!    .rx_queue_size(4096)
//!    .bind_flags(BindFlags::XDP_COPY) // Check your driver to see if you can use ZERO_COPY
//!    .xdp_flags(XdpFlags::XDP_FLAGS_SKB_MODE) // Check your driver to see if you can use DRV_MODE
//!    .build()
//!    .unwrap();
//!
//! let n_tx_frames = umem_config.frame_count() / 2;
//!
//! let mut xsk = Xsk2::new(&ifname, 0, umem_config, socket_config, n_tx_frames as usize);
//!
//! // Sending a packet
//! let pkt: Vec<u8> = vec![];
//! xsk.send(&pkt);
//!
//! // Receiving a packet
//! let (recvd_pkt, len) = xsk.recv().expect("failed to recv");
//! ```

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub mod socket;

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub mod umem;

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub mod xsk;

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
mod util;

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub use socket::{
    BindFlags, LibbpfFlags, RxQueue, Socket, SocketConfig, SocketConfigBuilder, TxQueue, XdpFlags,
};

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub use umem::{
    AccessError, CompQueue, DataError, FillQueue, Frame, Umem, UmemBuilder, UmemConfig,
    UmemConfigBuilder, WriteError,
};

#[cfg(test)]
mod tests {
    use std::mem;

    #[test]
    fn ensure_usize_and_u64_are_same_size() {
        assert_eq!(mem::size_of::<usize>(), mem::size_of::<u64>());
    }
}

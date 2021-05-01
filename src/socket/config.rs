use bitflags::bitflags;
use libbpf_sys::{XSK_RING_CONS__DEFAULT_NUM_DESCS, XSK_RING_PROD__DEFAULT_NUM_DESCS};
use std::{error::Error, fmt};

use crate::util;

bitflags! {
    pub struct LibbpfFlags: u32 {
        const XSK_LIBBPF_FLAGS__INHIBIT_PROG_LOAD = 1;
    }
}

bitflags! {
    /// AF_XDP Configuration flags
    pub struct XdpFlags: u32 {
        const XDP_FLAGS_UPDATE_IF_NOEXIST = 1;
        /// A socket buffer is allocated for each packet processed.
        /// This is a fall-back in case your driver does not support XDP
        /// and is slower than DRV_MODE
        const XDP_FLAGS_SKB_MODE = 2;
        /// Enables XDP driver mode, which bypasses the socket buffer path.
        const XDP_FLAGS_DRV_MODE = 4;
        const XDP_FLAGS_HW_MODE = 8;
        const XDP_FLAGS_REPLACE = 16;
    }
}

bitflags! {
    /// AF_XDP Configuration flags
    pub struct BindFlags: u16 {
        /// Allows you to bind multiple sockets to the same Umem
        const XDP_SHARED_UMEM = 1;
        /// Packets are copied from userspace, SLOW
        const XDP_COPY = 2;
        /// Packets are NOT copied from userspace, FAST
        const XDP_ZEROCOPY = 4;
        /// If this flag is set after the bind call,
        /// the kernel must be woken up via a system call in order
        /// to process packets
        const XDP_USE_NEED_WAKEUP = 8;
    }
}

#[derive(Debug)]
pub enum SocketConfigError {
    TxSizeInvalid { reason: &'static str },
    RxSizeInvalid { reason: &'static str },
}

impl fmt::Display for SocketConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use SocketConfigError::*;
        let reason = match self {
            TxSizeInvalid { reason } => reason,
            RxSizeInvalid { reason } => reason,
        };
        write!(f, "{}", reason)
    }
}

impl Error for SocketConfigError {}

/// Config for a [Socket](struct.Socket.html).
///
/// `rx_queue_size` and `tx_queue_size` must be powers of two.
#[derive(Debug, Clone, PartialEq)]
pub struct SocketConfig {
    rx_queue_size: u32,
    tx_queue_size: u32,
    libbpf_flags: LibbpfFlags,
    xdp_flags: XdpFlags,
    bind_flags: BindFlags,
}

impl SocketConfig {
    pub fn new(
        rx_queue_size: u32,
        tx_queue_size: u32,
        libbpf_flags: LibbpfFlags,
        xdp_flags: XdpFlags,
        bind_flags: BindFlags,
    ) -> Result<Self, SocketConfigError> {
        if !util::is_pow_of_two(rx_queue_size) {
            return Err(SocketConfigError::RxSizeInvalid {
                reason: "rx queue size must be a power of two",
            });
        }
        if !util::is_pow_of_two(tx_queue_size) {
            return Err(SocketConfigError::TxSizeInvalid {
                reason: "tx queue size must be a power of two",
            });
        }

        Ok(SocketConfig {
            rx_queue_size,
            tx_queue_size,
            libbpf_flags,
            xdp_flags,
            bind_flags,
        })
    }

    pub fn rx_queue_size(&self) -> u32 {
        self.rx_queue_size
    }

    pub fn tx_queue_size(&self) -> u32 {
        self.tx_queue_size
    }

    pub fn libbpf_flags(&self) -> &LibbpfFlags {
        &self.libbpf_flags
    }

    pub fn xdp_flags(&self) -> &XdpFlags {
        &self.xdp_flags
    }

    pub fn bind_flags(&self) -> &BindFlags {
        &self.bind_flags
    }
}

impl Default for SocketConfig {
    /// Default configuration based on constants set in the `libbpf`
    /// library with none of the flags set.
    fn default() -> Self {
        Self {
            rx_queue_size: XSK_RING_CONS__DEFAULT_NUM_DESCS,
            tx_queue_size: XSK_RING_PROD__DEFAULT_NUM_DESCS,
            libbpf_flags: LibbpfFlags::empty(),
            xdp_flags: XdpFlags::empty(),
            bind_flags: BindFlags::empty(),
        }
    }
}

/// Builder for [SocketConfig](struct.SocketConfig.html).
#[derive(Debug, Clone)]
pub struct SocketConfigBuilder {
    rx_queue_size: u32,
    tx_queue_size: u32,
    libbpf_flags: LibbpfFlags,
    xdp_flags: XdpFlags,
    bind_flags: BindFlags,
}

impl Default for SocketConfigBuilder {
    /// Default configuration based on constants set in the `libbpf`
    /// library with none of the flags set.
    fn default() -> Self {
        Self {
            rx_queue_size: XSK_RING_CONS__DEFAULT_NUM_DESCS,
            tx_queue_size: XSK_RING_PROD__DEFAULT_NUM_DESCS,
            libbpf_flags: LibbpfFlags::empty(),
            xdp_flags: XdpFlags::empty(),
            bind_flags: BindFlags::empty(),
        }
    }
}

impl SocketConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rx_queue_size(&mut self, rx_queue_size: u32) -> &mut Self {
        self.rx_queue_size = rx_queue_size;
        self
    }

    pub fn tx_queue_size(&mut self, tx_queue_size: u32) -> &mut Self {
        self.tx_queue_size = tx_queue_size;
        self
    }

    pub fn libbpf_flags(&mut self, libbpf_flags: LibbpfFlags) -> &mut Self {
        self.libbpf_flags = libbpf_flags;
        self
    }

    pub fn xdp_flags(&mut self, xdp_flags: XdpFlags) -> &mut Self {
        self.xdp_flags = xdp_flags;
        self
    }

    pub fn bind_flags(&mut self, bind_flags: BindFlags) -> &mut Self {
        self.bind_flags = bind_flags;
        self
    }

    pub fn build(&self) -> Result<SocketConfig, SocketConfigError> {
        SocketConfig::new(
            self.rx_queue_size,
            self.tx_queue_size,
            self.libbpf_flags,
            self.xdp_flags,
            self.bind_flags,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_config_builder_test() {
        let socket_config_default = SocketConfig::default();

        let socket_config = SocketConfigBuilder::new()
            .rx_queue_size(socket_config_default.rx_queue_size)
            .tx_queue_size(socket_config_default.tx_queue_size)
            .libbpf_flags(socket_config_default.libbpf_flags)
            .xdp_flags(socket_config_default.xdp_flags)
            .bind_flags(socket_config_default.bind_flags)
            .build()
            .expect("failed to create socket config");

        assert_eq!(socket_config_default, socket_config);
    }
}

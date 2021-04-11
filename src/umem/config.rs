use libbpf_sys::{
    XSK_RING_CONS__DEFAULT_NUM_DESCS, XSK_RING_PROD__DEFAULT_NUM_DESCS,
    XSK_UMEM__DEFAULT_FRAME_HEADROOM, XSK_UMEM__DEFAULT_FRAME_SIZE,
};
use std::{error::Error, fmt};

use crate::util;

#[derive(Debug)]
pub enum UmemConfigError {
    FrameCountInvalid { reason: &'static str },
    CompSizeInvalid { reason: &'static str },
    FillSizeInvalid { reason: &'static str },
    FrameSizeInvalid { reason: &'static str },
}

impl fmt::Display for UmemConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use UmemConfigError::*;
        let reason = match self {
            FrameCountInvalid { reason } => reason,
            CompSizeInvalid { reason } => reason,
            FillSizeInvalid { reason } => reason,
            FrameSizeInvalid { reason } => reason,
        };
        write!(f, "{}", reason)
    }
}

impl Error for UmemConfigError {}

/// Config for a [Umem](struct.Umem.html) instance.
///
/// `fill_queue_size` and `comp_queue_size` must be powers of two and
/// frame size must not be less than `2048`. If you have set
/// `use_huge_pages` as `true` but are getting errors, check that the
/// `HugePages_Total` setting is non-zero when you run `cat
/// /proc/meminfo`.
///
/// It's worth noting that the specified `frame_size` is not
/// necessarily the buffer size that will be available to write data
/// into. Some of this will be eaten up by `XDP_PACKET_HEADROOM` and
/// any non-zero `frame_headroom`, so make sure to check that
/// `frame_size` is large enough to hold the data you with to transmit
/// (e.g. an ETH frame) plus `XDP_PACKET_HEADROOM + frame_headroom`
/// bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct UmemConfig {
    frame_count: u32,
    frame_size: u32,
    fill_queue_size: u32,
    comp_queue_size: u32,
    frame_headroom: u32,
    use_huge_pages: bool,
}

impl Default for UmemConfig {
    /// Default configuration based on constants set in the `libbpf` library.
    fn default() -> Self {
        Self {
            frame_count: 2048,
            frame_size: XSK_UMEM__DEFAULT_FRAME_SIZE,
            fill_queue_size: XSK_RING_PROD__DEFAULT_NUM_DESCS,
            comp_queue_size: XSK_RING_CONS__DEFAULT_NUM_DESCS,
            frame_headroom: XSK_UMEM__DEFAULT_FRAME_HEADROOM,
            use_huge_pages: false,
        }
    }
}

impl UmemConfig {
    pub fn new(
        frame_count: u32,
        frame_size: u32,
        fill_queue_size: u32,
        comp_queue_size: u32,
        frame_headroom: u32,
        use_huge_pages: bool,
    ) -> Result<Self, UmemConfigError> {
        if frame_count == 0 {
            return Err(UmemConfigError::FrameCountInvalid {
                reason: "frame count must be greater than 0",
            });
        }
        if !util::is_pow_of_two(fill_queue_size) {
            return Err(UmemConfigError::FillSizeInvalid {
                reason: "fill queue size must be a power of two",
            });
        }
        if !util::is_pow_of_two(comp_queue_size) {
            return Err(UmemConfigError::CompSizeInvalid {
                reason: "comp queue size must be a power of two",
            });
        }
        if frame_size < 2048 {
            return Err(UmemConfigError::FrameSizeInvalid {
                reason: "frame size must be greater than or equal to 2048",
            });
        }

        Ok(UmemConfig {
            frame_count,
            frame_size,
            fill_queue_size,
            comp_queue_size,
            frame_headroom,
            use_huge_pages,
        })
    }

    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    pub fn fill_queue_size(&self) -> u32 {
        self.fill_queue_size
    }

    pub fn comp_queue_size(&self) -> u32 {
        self.comp_queue_size
    }

    pub fn frame_headroom(&self) -> u32 {
        self.frame_headroom
    }

    pub fn use_huge_pages(&self) -> bool {
        self.use_huge_pages
    }

    pub fn umem_len(&self) -> usize {
        // Know size_of<usize> = size_of<u64> due to compilation
        // flags so 'as' upcast from u32 is ok
        (self.frame_count as usize)
            .checked_mul(self.frame_size as usize)
            .unwrap()
    }
}

/// Builder for UmemConfig for a [UmemConfig](struct.UmemConfig.html).
#[derive(Debug, Clone)]
pub struct UmemConfigBuilder {
    frame_count: u32,
    frame_size: u32,
    fill_queue_size: u32,
    comp_queue_size: u32,
    frame_headroom: u32,
    use_huge_pages: bool,
}

impl Default for UmemConfigBuilder {
    /// Default configuration based on constants set in the `libbpf` library.
    fn default() -> Self {
        Self {
            frame_count: 2048,
            frame_size: XSK_UMEM__DEFAULT_FRAME_SIZE,
            fill_queue_size: XSK_RING_PROD__DEFAULT_NUM_DESCS,
            comp_queue_size: XSK_RING_CONS__DEFAULT_NUM_DESCS,
            frame_headroom: XSK_UMEM__DEFAULT_FRAME_HEADROOM,
            use_huge_pages: false,
        }
    }
}

impl UmemConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn frame_count(&mut self, frame_count: u32) -> &mut Self {
        self.frame_count = frame_count;
        self
    }

    pub fn frame_size(&mut self, frame_size: u32) -> &mut Self {
        self.frame_size = frame_size;
        self
    }

    pub fn fill_queue_size(&mut self, fill_queue_size: u32) -> &mut Self {
        self.fill_queue_size = fill_queue_size;
        self
    }

    pub fn comp_queue_size(&mut self, comp_queue_size: u32) -> &mut Self {
        self.comp_queue_size = comp_queue_size;
        self
    }

    pub fn frame_headroom(&mut self, frame_headroom: u32) -> &mut Self {
        self.frame_headroom = frame_headroom;
        self
    }

    pub fn use_huge_pages(&mut self, use_huge_pages: bool) -> &mut Self {
        self.use_huge_pages = use_huge_pages;
        self
    }

    pub fn build(&self) -> Result<UmemConfig, UmemConfigError> {
        UmemConfig::new(
            self.frame_count,
            self.frame_size,
            self.fill_queue_size,
            self.comp_queue_size,
            self.frame_headroom,
            self.use_huge_pages,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn umem_len_doesnt_panic_when_frame_count_and_size_are_u32_max() {
        UmemConfig::new(u32::MAX, u32::MAX, 8, 8, 0, false)
            .unwrap()
            .umem_len();
    }

    #[test]
    fn umem_config_builder_test() {
        let umem_config_default = UmemConfig::default();

        let umem_config = UmemConfigBuilder::new()
            .frame_count(umem_config_default.frame_count)
            .frame_size(umem_config_default.frame_size)
            .fill_queue_size(umem_config_default.fill_queue_size)
            .comp_queue_size(umem_config_default.comp_queue_size)
            .frame_headroom(umem_config_default.frame_headroom)
            .use_huge_pages(umem_config_default.use_huge_pages)
            .build()
            .expect("failed to build umem config");

        assert_eq!(umem_config_default, umem_config);
    }
}

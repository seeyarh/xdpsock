use errno::errno;
use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_umem, xsk_umem_config, XDP_PACKET_HEADROOM};
use std::sync::Arc;
use std::{convert::TryInto, error::Error, fmt, io, mem::MaybeUninit, ptr};

use crate::socket::{self, Fd};

use super::{config::UmemConfig, mmap::MmapArea};

/// Represents where in the tx/rx lifecycle a Frame is
#[derive(Debug, Clone, PartialEq)]
pub enum FrameStatus {
    Free,
    Filled,
    OnTxQueue,
    OnFillQueue,
}

impl FrameStatus {
    pub fn is_free(&self) -> bool {
        match self {
            FrameStatus::Free => true,
            _ => false,
        }
    }
}

/// Describes a UMEM frame's address and size of its current contents.
///
/// The `addr` field is an offset in bytes from the start of the UMEM
/// and corresponds to some point within a frame. The `len` field
/// describes the length (in bytes) of any data stored in that frame,
/// starting from `addr`.
///
/// If sending data, the `len` field will need to be set by the user
/// before transmitting via the [TxQueue](struct.TxQueue.html).
/// Otherwise when reading received frames using the
/// [RxQueue](struct.RxQueue.html), the `len` field will be set by the
/// kernel and dictates the number of bytes the user should read from
/// the UMEM.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    addr: usize,
    len: usize,
    options: u32,
    mtu: usize,
    mmap_area: Arc<MmapArea>,
    pub status: FrameStatus,
}

impl Frame {
    #[inline]
    pub fn addr(&self) -> usize {
        self.addr
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn options(&self) -> u32 {
        self.options
    }

    /// Set the frame descriptor's address. This determines where in
    /// the UMEM it references.
    ///
    /// Manual setting shouldn't generally be required is likely best
    /// avoided since the setting of addresses is handled by the
    /// library, however it may be needed if writing straight to a
    /// region in UMEM via
    /// [umem_region_mut](struct.Umem.html#method.umem_region_mut) or
    /// [umem_region_mut_checked](struct.Umem.html#method.umem_region_mut_checked)
    #[inline]
    pub fn set_addr(&mut self, addr: usize) {
        self.addr = addr
    }

    /// Set the frame descriptor's length. This should equal the
    /// length of the data stored at `addr`.
    ///
    /// Once data has been written to the UMEM region starting at
    /// `addr`, this `FrameDesc`'s length must be updated before
    /// handing it over to the kernel to be transmitted to ensure the
    /// correct number of bytes are sent.
    ///
    /// Manual setting shouldn't generally be required and if copying
    /// packets to UMEM it's better to use
    /// [write_to_umem](struct.Umem.html#method.write_to_umem) or
    /// [write_to_umem_checked](struct.Umem.html#method.write_to_umem_checked)
    /// which will handle setting the frame descriptor length, however
    /// it may be needed if writing to a UMEM region manually (see
    /// `set_addr`) or if, for example, the data you want to send is
    /// already at `addr` and you just need to set the length before
    /// transmission.
    #[inline]
    pub fn set_len(&mut self, len: usize) {
        self.len = len
    }

    /// Set the frame descriptor options.
    #[inline]
    pub fn set_options(&mut self, options: u32) {
        self.options = options
    }

    /// Check if `data` is ok to write to the UMEM for transmission.
    #[inline]
    pub fn is_access_valid(&self, len: usize) -> Result<(), AccessError> {
        if len > self.len {
            return Err(AccessError::CrossesFrameBoundary {
                addr: self.addr,
                len,
            });
        } else {
            Ok(())
        }
    }

    /// Check if `data` is ok to write to the UMEM for transmission.
    #[inline]
    pub fn is_data_valid(&self, data: &[u8]) -> Result<(), DataError> {
        // Check if data is transmissable
        if data.len() > self.mtu {
            return Err(DataError::SizeExceedsMtu {
                data_len: data.len(),
                mtu: self.mtu,
            });
        }

        Ok(())
    }

    /// Return a reference to the UMEM region starting at `addr` of
    /// length `len`.
    ///
    /// This does not check that the region accessed makes sense and
    /// may cause undefined behaviour if used improperly. An example
    /// of potential misuse is referencing a region that extends
    /// beyond the end of the UMEM.
    ///
    /// Apart from the memory considerations, this function is also
    /// `unsafe` as there is no guarantee the kernel isn't also
    /// reading from or writing to the same region.
    #[inline]
    pub unsafe fn read_from_umem(&self, len: usize) -> &[u8] {
        self.mmap_area.mem_range(self.addr, len)
    }

    /// Checked version of `umem_region_ref`. Ensures that the
    /// referenced region is contained within a single frame of the
    /// UMEM.
    #[inline]
    pub unsafe fn read_from_umem_checked(&self, len: usize) -> Result<&[u8], AccessError> {
        self.is_access_valid(len)?;
        Ok(self.mmap_area.mem_range(self.addr, len))
    }

    /// Copy `data` to the region starting at `frame_desc.addr`, and
    /// set `frame_desc.len` when done.
    ///
    /// This does no checking that the region written to makes sense
    /// and may cause undefined behaviour if used improperly. An
    /// example of potential misuse is writing beyond the end of the
    /// UMEM, or if `data` is large then potentially writing across
    /// frame boundaries.
    ///
    /// Apart from the considerations around writing to memory, this
    /// function is also `unsafe` as there is no guarantee the kernel
    /// isn't also reading from or writing to the same region.
    #[inline]
    pub unsafe fn write_to_umem(&mut self, data: &[u8]) {
        let data_len = data.len();

        if data_len > 0 {
            let umem_region = self.mmap_area.mem_range_mut(&self.addr(), &data_len);

            umem_region[..data_len].copy_from_slice(data);
        }

        self.set_len(data_len);
    }

    /// Checked version of `write_to_umem_frame`. Ensures that a
    /// successful write is completely contained within a single frame
    /// of the UMEM.
    #[inline]
    pub unsafe fn write_to_umem_checked(&mut self, data: &[u8]) -> Result<(), WriteError> {
        let data_len = data.len();

        if data_len > 0 {
            self.is_data_valid(data).map_err(|e| WriteError::Data(e))?;

            let umem_region = self.mmap_area.mem_range_mut(&self.addr(), &data_len);

            umem_region[..data_len].copy_from_slice(data);
            log::debug!("write_to_umem {:?}", umem_region);
        }

        self.set_len(data_len);

        Ok(())
    }

    /// Return a reference to the UMEM region starting at `addr` of
    /// length `len`.
    ///
    /// This does not check that the region accessed makes sense and
    /// may cause undefined behaviour if used improperly. An example
    /// of potential misuse is referencing a region that extends
    /// beyond the end of the UMEM.
    ///
    /// Apart from the memory considerations, this function is also
    /// `unsafe` as there is no guarantee the kernel isn't also
    /// reading from or writing to the same region.
    ///
    /// If data is written to a frame, the length on the corresponding
    /// [FrameDesc](struct.FrameDesc.html) for `addr` must be updated
    /// before submitting to the [TxQueue](struct.TxQueue.html). This
    /// ensures the correct number of bytes are sent. Use
    /// `write_to_umem` or `write_to_umem_checked` to avoid the
    /// overhead of updating the frame descriptor.
    #[inline]
    pub unsafe fn umem_region_mut(&mut self, len: usize) -> &mut [u8] {
        self.mmap_area.mem_range_mut(&self.addr, &len)
    }

    /// Checked version of `umem_region_mut`. Ensures the requested
    /// region lies within a single frame.
    #[inline]
    pub unsafe fn umem_region_mut_checked(&mut self, len: usize) -> Result<&mut [u8], AccessError> {
        Ok(self.mmap_area.mem_range_mut(&self.addr, &len))
    }
}

/// Initial step for building a UMEM. This creates the underlying
/// `mmap` area.
pub struct UmemBuilder {
    config: UmemConfig,
}

/// Use the `mmap`'d region to create the UMEM.
pub struct UmemBuilderWithMmap {
    config: UmemConfig,
    mmap_area: MmapArea,
}

#[derive(Debug)]
struct XskUmem(*mut xsk_umem);

unsafe impl Send for XskUmem {}

impl Drop for XskUmem {
    fn drop(&mut self) {
        let err = unsafe { libbpf_sys::xsk_umem__delete(self.0) };
        log::debug!("drop_umem: dropping umem err = {}", err);

        log::debug!(
            "drop_umem: dropping umem err = {}",
            std::io::Error::from_raw_os_error(err)
        );

        if err != 0 {
            log::error!("xsk_umem__delete({:?}) failed: {}", self.0, errno());
        }
    }
}

/// A region of virtual contiguous memory divided into equal-sized
/// frames.  It provides the underlying working memory for an AF_XDP
/// socket.
#[derive(Debug)]
pub struct Umem {
    config: UmemConfig,
    mtu: usize,
    inner: Box<XskUmem>,
}

impl UmemBuilder {
    /// Allocate a memory region for the UMEM.
    ///
    /// Before we can create the UMEM we first need to allocate a
    /// chunk of memory, which will eventually be split up into
    /// frames. We do this with a call to `mmap`, requesting a read +
    /// write protected anonymous memory region.
    pub fn create_mmap(self) -> io::Result<UmemBuilderWithMmap> {
        let mmap_area = MmapArea::new(self.config.umem_len(), self.config.use_huge_pages())?;

        Ok(UmemBuilderWithMmap {
            config: self.config,
            mmap_area,
        })
    }
}

impl UmemBuilderWithMmap {
    /// Using the allocated memory region, create the UMEM.
    ///
    /// Once we've successfully requested a region of memory, create
    /// the UMEM with it by splitting the memory region into frames
    /// and creating the [FillQueue](struct.FillQueue.html) and
    /// [CompQueue](struct.CompQueue.html).
    pub fn create_umem(mut self) -> io::Result<(Umem, FillQueue, CompQueue, Vec<Frame>)> {
        let umem_create_config = xsk_umem_config {
            fill_size: self.config.fill_queue_size(),
            comp_size: self.config.comp_queue_size(),
            frame_size: self.config.frame_size(),
            frame_headroom: self.config.frame_headroom(),
            flags: 0,
        };

        let mut umem_ptr: *mut xsk_umem = ptr::null_mut();
        let mut fq_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::zeroed();
        let mut cq_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::zeroed();

        let err = unsafe {
            libbpf_sys::xsk_umem__create(
                &mut umem_ptr,
                self.mmap_area.as_mut_ptr(),
                self.mmap_area.len() as u64,
                fq_ptr.as_mut_ptr(),
                cq_ptr.as_mut_ptr(),
                &umem_create_config,
            )
        };

        if err != 0 {
            let e = errno::errno();
            return Err(io::Error::from_raw_os_error(e.0));
        }

        // Upcasting u32 -> size_of<usize> = size_of<u64> is ok, the
        // latter equality being guaranteed by the crate's top level
        // conditional compilation flags (see lib.rs)
        let frame_size = self.config.frame_size() as usize;
        let frame_count = self.config.frame_count() as usize;

        let frame_headroom = self.config.frame_headroom() as usize;
        let xdp_packet_headroom = XDP_PACKET_HEADROOM as usize;
        let mtu = frame_size - (xdp_packet_headroom + frame_headroom);

        let mut frames: Vec<Frame> = Vec::with_capacity(frame_count);

        let mmap = Arc::new(self.mmap_area);

        for i in 0..frame_count {
            let addr = i * frame_size;
            let len = 0;
            let options = 0;

            let frame = Frame {
                addr,
                len,
                options,
                mtu,
                mmap_area: mmap.clone(),
                status: FrameStatus::Free,
            };

            frames.push(frame);
        }

        let fill_queue = FillQueue {
            size: self.config.fill_queue_size(),
            inner: unsafe { Box::new(fq_ptr.assume_init()) },
        };

        let comp_queue = CompQueue {
            size: self.config.comp_queue_size(),
            inner: unsafe { Box::new(cq_ptr.assume_init()) },
        };

        let umem = Umem {
            config: self.config,
            mtu,
            inner: Box::new(XskUmem(umem_ptr)),
        };

        Ok((umem, fill_queue, comp_queue, frames))
    }
}

impl Umem {
    pub fn builder(config: UmemConfig) -> UmemBuilder {
        UmemBuilder { config }
    }

    /// Config used for building the UMEM.
    pub fn config(&self) -> &UmemConfig {
        &self.config
    }

    /// The maximum transmission unit, this determines
    /// the largest packet that may be sent.
    ///
    /// Equal to `frame_size - (XDP_PACKET_HEADROOM + frame_headroom)`.
    #[inline]
    pub fn mtu(&self) -> usize {
        self.mtu
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut xsk_umem {
        unsafe { self.inner.0.as_mut().expect("failed to get mut umem ptr") }
    }
}

/// Used to transfer ownership of UMEM frames from user-space to
/// kernel-space.
///
/// These frames will be used to receive packets, and will eventually
/// be returned via the [RxQueue](struct.RxQueue.html).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-fill-ring)
#[derive(Debug)]
pub struct FillQueue {
    size: u32,
    inner: Box<xsk_ring_prod>,
}

impl FillQueue {
    /// Let the kernel know that the frames in `descs` may be used to
    /// receive data.
    ///
    /// This function is marked `unsafe` as it is possible to cause a
    /// data race by simultaneously submitting the same frame
    /// descriptor to the fill ring and the Tx ring, for example.
    /// Once the frames have been submitted they should not be used
    /// again until consumed again via the
    /// [RxQueue](struct.RxQueue.html).
    ///
    /// Note that if the length of `descs` is greater than the number
    /// of available spaces on the underlying ring buffer then no
    /// frames at all will be handed over to the kernel.
    ///
    /// This returns the number of frames submitted to the kernel. Due
    /// to the constraint mentioned in the above paragraph, this
    /// should always be the length of `descs` or `0`.
    #[inline]
    pub unsafe fn produce(&mut self, descs: &[Frame]) -> usize {
        // usize <-> u64 'as' conversions are ok as the crate's top
        // level conditional compilation flags (see lib.rs) guarantee
        // that size_of<usize> = size_of<u64>
        let nb = descs.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb, &mut idx);

        if cnt > 0 {
            for desc in descs.iter().take(cnt.try_into().unwrap()) {
                *libbpf_sys::_xsk_ring_prod__fill_addr(self.inner.as_mut(), idx) = desc.addr as u64;

                idx += 1;
            }

            libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt);
        }

        cnt.try_into().unwrap()
    }

    /// Same as `produce` but wake up the kernel (if required) to let
    /// it know there are frames available that may be used to receive
    /// data.
    ///
    /// For more details see the
    /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
    ///
    /// This function is marked `unsafe` for the same reasons that
    /// `produce` is `unsafe`.
    #[inline]
    pub unsafe fn produce_and_wakeup(
        &mut self,
        descs: &[Frame],
        socket_fd: &mut Fd,
        poll_timeout: i32,
    ) -> io::Result<usize> {
        let cnt = self.produce(descs);

        if cnt > 0 && self.needs_wakeup() {
            self.wakeup(socket_fd, poll_timeout)?;
        }

        Ok(cnt)
    }

    /// Wake up the kernel to let it know it can continue using the
    /// fill ring to process received data.
    ///
    /// See `produce_and_wakeup` for link to docs with further
    /// explanation.
    #[inline]
    pub fn wakeup(&self, fd: &mut Fd, poll_timeout: i32) -> io::Result<()> {
        socket::poll_read(fd, poll_timeout)?;
        Ok(())
    }

    /// Check if the libbpf `NEED_WAKEUP` flag is set on the fill
    /// ring.  If so then this means a call to `wakeup` will be
    /// required to continue processing received data with the fill
    /// ring.
    ///
    /// See `produce_and_wakeup` for link to docs with further
    /// explanation.
    #[inline]
    pub fn needs_wakeup(&self) -> bool {
        unsafe { libbpf_sys::_xsk_ring_prod__needs_wakeup(self.inner.as_ref()) != 0 }
    }
}

unsafe impl Send for FillQueue {}

/// Used to transfer ownership of UMEM frames from kernel-space to
/// user-space.
///
/// Frames received in this queue are those that have been sent via
/// the [TxQueue](struct.TxQueue.html).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-completion-ring)
#[derive(Debug)]
pub struct CompQueue {
    size: u32,
    inner: Box<xsk_ring_cons>,
}

impl CompQueue {
    /// Update `descs` with frames whose contents have been sent
    /// (after submission via the [TxQueue](struct.TxQueue.html) and
    /// may now be used again.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `descs`.  Entries will be updated sequentially
    /// from the start of `descs` until the end.  Returns the number
    /// of elements of `descs` which have been updated.
    ///
    /// Free frames should be added back on to either the
    /// [FillQueue](struct.FillQueue.html) for data receipt or the
    /// [TxQueue](struct.TxQueue.html) for data transmission.
    #[inline]
    pub fn consume(&mut self, n_frames: u64, free_frames: &mut [u64]) -> u64 {
        let mut idx: u32 = 0;

        let cnt =
            unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), n_frames, &mut idx) };

        for i in 0..cnt {
            let addr: u64 = unsafe {
                *libbpf_sys::_xsk_ring_cons__comp_addr(self.inner.as_mut(), idx + i as u32)
            };
            free_frames[i as usize] = addr;
        }

        unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt) };

        cnt
    }
}

unsafe impl Send for CompQueue {}

/// UMEM access errors
#[derive(Debug)]
pub enum AccessError {
    /// Attempted to access a region with zero length.
    NullRegion,
    /// Attempted to access a region outside of the UMEM.
    RegionOutOfBounds {
        addr: usize,
        len: usize,
        umem_len: usize,
    },
    /// Attempted to access a region that intersects with two or more
    /// frames.
    CrossesFrameBoundary { addr: usize, len: usize },
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use AccessError::*;
        match self {
            NullRegion => write!(f, "region has zero length"),
            RegionOutOfBounds {
                addr,
                len,
                umem_len,
            } => write!(
                f,
                "UMEM region [{}, {}] is out of bounds (UMEM length is {})",
                addr,
                addr + (len - 1),
                umem_len
            ),
            CrossesFrameBoundary { addr, len } => write!(
                f,
                "UMEM region [{}, {}] intersects with more then one frame",
                addr,
                addr + (len - 1),
            ),
        }
    }
}

impl Error for AccessError {}

/// Data related errors
#[derive(Debug)]
pub enum DataError {
    /// Size of data written to UMEM for tx exceeds the MTU.
    SizeExceedsMtu { data_len: usize, mtu: usize },
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::SizeExceedsMtu { data_len, mtu } => write!(
                f,
                "data length ({} bytes) cannot be greater than the MTU ({} bytes)",
                data_len, mtu
            ),
        }
    }
}

impl Error for DataError {}

/// Errors that may occur when writing data to the UMEM.
#[derive(Debug)]
pub enum WriteError {
    Access(AccessError),
    Data(DataError),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use WriteError::*;
        match self {
            Access(access_err) => write!(f, "{}", access_err),
            Data(data_err) => write!(f, "{}", data_err),
        }
    }
}

impl Error for WriteError {}

#[cfg(test)]
mod tests {
    use rand;

    use serial_test::serial;

    use super::*;
    use crate::umem::UmemConfig;

    const FRAME_COUNT: u32 = 8;
    const FRAME_SIZE: u32 = 2048;

    fn generate_random_bytes(len: u32) -> Vec<u8> {
        (0..len).map(|_| rand::random::<u8>()).collect()
    }

    fn umem_config() -> UmemConfig {
        UmemConfig::new(FRAME_COUNT, FRAME_SIZE, 4, 4, 0, false).unwrap()
    }

    fn umem() -> (Umem, FillQueue, CompQueue, Vec<Frame>) {
        let config = umem_config();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM")
    }

    #[test]
    #[serial]
    fn umem_create_succeeds_when_frame_count_is_one() {
        let config = UmemConfig::new(1, 4096, 4, 4, 0, false).unwrap();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");
    }

    #[test]
    #[serial]
    fn umem_create_succeeds_when_fill_size_is_one() {
        let config = UmemConfig::new(16, 4096, 1, 4, 0, false).unwrap();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");
    }

    #[test]
    #[serial]
    fn umem_create_succeeds_when_comp_size_is_one() {
        let config = UmemConfig::new(16, 4096, 4, 1, 0, false).unwrap();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");
    }

    #[test]
    #[serial]
    #[should_panic]
    fn umem_create_fails_when_frame_size_is_lt_2048() {
        let config = UmemConfig::new(1, 2047, 4, 4, 0, false).unwrap();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");
    }

    #[test]
    #[serial]
    fn mtu_is_correct() {
        let config = UmemConfig::new(1, 2048, 4, 4, 512, false).unwrap();

        let (umem, _fq, _cq, _frame_descs) = Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");

        assert_eq!(umem.mtu(), (2048 - XDP_PACKET_HEADROOM - 512) as usize);
    }

    #[test]
    #[serial]
    fn umem_access_checks_ok() {
        /*
        let (_umem, _fq, _cq, frames) = umem();

        let max_len = FRAME_SIZE as usize;

        assert!(frames[0].is_access_valid(1).is_ok());

        assert!(frames[0].is_access_valid(max_len).is_ok());

        assert!(matches!(
            frames[0].is_access_valid(max_len + 1),
            Err(AccessError::CrossesFrameBoundary { .. })
        ));

        let last_frame_addr = ((FRAME_COUNT - 1) * FRAME_SIZE) as usize;

        assert!(frames[frames.len() - 1].is_access_valid(max_len).is_ok());

        assert!(matches!(
            frames[frames.len() - 1].is_access_valid(max_len + 1),
            Err(AccessError::CrossesFrameBoundary { .. })
        ));
        */
    }

    #[test]
    #[serial]
    fn data_checks_ok() {
        let (_umem, _fq, _cq, frames) = umem();

        // Empty data ok
        let empty_data: Vec<u8> = Vec::new();

        assert!(frames[0].is_data_valid(&empty_data).is_ok());

        let mtu = FRAME_SIZE - XDP_PACKET_HEADROOM;

        // Data within mtu ok
        let data = generate_random_bytes(mtu - 1);

        assert!(frames[0].is_data_valid(&data).is_ok());

        // Data exactly frame size is ok
        let data = generate_random_bytes(mtu);

        assert!(frames[0].is_data_valid(&data).is_ok());

        // Data greater than frame size fails
        let data = generate_random_bytes(mtu + 1);

        assert!(matches!(
            frames[0].is_data_valid(&data),
            Err(DataError::SizeExceedsMtu { .. })
        ));
    }

    #[test]
    #[serial]
    fn write_no_data_to_umem() {
        let (mut _umem, _fq, _cq, mut frames) = umem();

        let data = [];

        unsafe {
            frames[0].write_to_umem_checked(&data[..]).unwrap();
        }

        assert_eq!(frames[0].len(), 0);
    }

    #[test]
    #[serial]
    fn write_to_umem_frame_then_read_small_byte_array() {
        let (mut _umem, _fq, _cq, mut frames) = umem();

        let data = [b'H', b'e', b'l', b'l', b'o'];

        unsafe {
            frames[0].write_to_umem_checked(&data[..]).unwrap();
        }

        assert_eq!(frames[0].len(), 5);

        let umem_region = unsafe { frames[0].read_from_umem_checked(frames[0].len).unwrap() };

        assert_eq!(data, umem_region[..data.len()]);
    }

    #[test]
    #[serial]
    fn write_max_bytes_to_neighbouring_umem_frames() {
        let (mut _umem, _fq, _cq, mut frames) = umem();

        let data_len = FRAME_SIZE;

        // Create random data and write to adjacent frames
        let fst_data = generate_random_bytes(data_len);
        let snd_data = generate_random_bytes(data_len);

        unsafe {
            let umem_region = frames[0]
                .umem_region_mut_checked(data_len as usize)
                .unwrap();

            umem_region.copy_from_slice(&fst_data[..]);
            frames[0].set_len(data_len as usize);

            let umem_region = frames[1]
                .umem_region_mut_checked(data_len as usize)
                .unwrap();

            umem_region.copy_from_slice(&snd_data[..]);
            frames[1].set_len(data_len as usize);
        }

        let fst_frame_ref = unsafe { frames[0].read_from_umem(frames[0].len()) };

        let snd_frame_ref = unsafe { frames[1].read_from_umem(frames[1].len()) };

        // Check that they are indeed the samelet fst_frame_ref = umem.frame_ref_at_addr(&fst_addr).unwrap();
        assert_eq!(fst_data[..], fst_frame_ref[..fst_data.len()]);
        assert_eq!(snd_data[..], snd_frame_ref[..snd_data.len()]);

        // Ensure there are no gaps and the frames lie snugly
        let mem_len = (FRAME_SIZE * 2) as usize;

        let mem_range = unsafe { frames[0].mmap_area.mem_range(0, mem_len) };

        let mut data_vec = Vec::with_capacity(mem_len);

        data_vec.extend_from_slice(&fst_data);
        data_vec.extend_from_slice(&snd_data);

        assert_eq!(&data_vec[..], mem_range);
    }
}

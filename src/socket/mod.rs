use libbpf_sys::{
    xsk_ring_cons, xsk_ring_prod, xsk_socket, xsk_socket_config, XDP_FLAGS_UPDATE_IF_NOEXIST,
    XDP_USE_NEED_WAKEUP,
};
use libc::{EAGAIN, EBUSY, ENETDOWN, ENOBUFS, MSG_DONTWAIT};
use std::{cmp, collections::VecDeque, convert::TryInto, ffi::CString, io, mem::MaybeUninit, ptr};

mod config;

use crate::{
    get_errno,
    poll::{poll_read, Milliseconds},
    umem::{FrameDesc, Umem},
};

pub struct Fd(i32);

impl Fd {
    pub(crate) fn descriptor(&self) -> i32 {
        self.0
    }
}

pub struct TxQueue {
    inner: Box<xsk_ring_prod>,
    socket_fd: Fd,
}

pub struct RxQueue {
    inner: Box<xsk_ring_cons>,
    socket_fd: Fd,
}

pub struct Socket {
    inner: Box<xsk_socket>,
    fd: Fd,
}

#[derive(Clone)]
pub struct SocketConfig {
    if_name: String,
    queue_id: u32,
    rx_queue_size: u32,
    tx_queue_size: u32,
}

impl SocketConfig {
    pub fn new(
        if_name: impl Into<String>,
        queue_id: u32,
        rx_queue_size: u32,
        tx_queue_size: u32,
    ) -> Self {
        let rx_queue_size = rx_queue_size.next_power_of_two();
        let tx_queue_size = tx_queue_size.next_power_of_two();

        SocketConfig {
            if_name: if_name.into(),
            queue_id,
            rx_queue_size,
            tx_queue_size,
        }
    }

    pub fn if_name(&self) -> &str {
        &self.if_name
    }

    pub fn queue_id(&self) -> u32 {
        self.queue_id
    }

    pub fn rx_queue_size(&self) -> u32 {
        self.rx_queue_size
    }

    pub fn tx_queue_size(&self) -> u32 {
        self.tx_queue_size
    }
}

impl Socket {
    pub fn new(config: SocketConfig, umem: &mut Umem) -> io::Result<(Socket, TxQueue, RxQueue)> {
        let socket_create_config = xsk_socket_config {
            rx_size: config.rx_queue_size,
            tx_size: config.tx_queue_size,
            xdp_flags: XDP_FLAGS_UPDATE_IF_NOEXIST,
            bind_flags: XDP_USE_NEED_WAKEUP as u16,
            libbpf_flags: 0,
        };

        let if_name = CString::new(config.if_name)
            .expect("Failed constructing CString from provided interface name");

        let mut xsk_ptr: *mut xsk_socket = ptr::null_mut();
        let mut tx_q_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::uninit();
        let mut rx_q_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::uninit();

        let err = unsafe {
            libbpf_sys::xsk_socket__create(
                &mut xsk_ptr,
                if_name.as_ptr(),
                config.queue_id,
                umem.as_mut_ptr(),
                rx_q_ptr.as_mut_ptr(),
                tx_q_ptr.as_mut_ptr(),
                &socket_create_config,
            )
        };

        if err != 0 {
            eprintln!("Failed to create and bind socket");
            return Err(io::Error::from_raw_os_error(err));
        }

        let fd = unsafe { libbpf_sys::xsk_socket__fd(xsk_ptr) };

        if fd < 0 {
            eprintln!("Failed while retrieving socket file descriptor");

            unsafe {
                libbpf_sys::xsk_socket__delete(xsk_ptr);
            }

            return Err(io::Error::from_raw_os_error(fd));
        }
        let socket = Socket {
            inner: unsafe { Box::from_raw(xsk_ptr) },
            fd: Fd(fd),
        };

        let tx_queue = TxQueue {
            inner: unsafe { Box::new(tx_q_ptr.assume_init()) },
            socket_fd: Fd(fd),
        };

        let rx_queue = RxQueue {
            inner: unsafe { Box::new(rx_q_ptr.assume_init()) },
            socket_fd: Fd(fd),
        };

        Ok((socket, tx_queue, rx_queue))
    }

    pub fn file_descriptor(&self) -> &Fd {
        &self.fd
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        unsafe {
            libbpf_sys::xsk_socket__delete(self.inner.as_mut());
        }
    }
}

impl RxQueue {
    pub fn consume(&mut self, descs: &mut VecDeque<FrameDesc>, nb: u64) -> u64 {
        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) };

        for _ in 0..cnt {
            unsafe {
                let recv_pkt_desc = libbpf_sys::_xsk_ring_cons__rx_desc(self.inner.as_mut(), idx);

                let addr = (*recv_pkt_desc).addr;
                let len = (*recv_pkt_desc).len;
                let options = (*recv_pkt_desc).options;

                descs.push_back(FrameDesc::new(addr, len, options))
            }
            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt) };
        }

        cnt
    }

    pub fn poll_and_consume(
        &mut self,
        descs: &mut VecDeque<FrameDesc>,
        nb: u64,
        poll_timeout: &Milliseconds,
    ) -> io::Result<Option<u64>> {
        match poll_read(&self.socket_fd, poll_timeout)? {
            true => Ok(Some(self.consume(descs, nb))),
            false => Ok(None),
        }
    }
}

impl TxQueue {
    pub fn produce(&mut self, descs: &mut VecDeque<FrameDesc>, nb: u64) -> u64 {
        // descs.len() returns usize, so should be ok to upcast to u64
        let nb = cmp::min(nb, descs.len().try_into().unwrap());

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb, &mut idx) };

        for _ in 0..cnt {
            // Ensured above that cnt <= nb <= descs.len()
            let desc = descs.pop_front().unwrap();

            unsafe {
                let send_frame_desc = libbpf_sys::_xsk_ring_prod__tx_desc(self.inner.as_mut(), idx);

                (*send_frame_desc).addr = desc.addr();
                (*send_frame_desc).len = desc.len();
                (*send_frame_desc).options = desc.options();
            }

            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt) };
        }

        cnt
    }

    pub fn produce_and_wakeup(
        &mut self,
        descs: &mut VecDeque<FrameDesc>,
        nb: u64,
    ) -> io::Result<u64> {
        let cnt = self.produce(descs, nb);

        if self.needs_wakeup() {
            let ret = unsafe {
                libc::sendto(
                    self.socket_fd.0,
                    ptr::null(),
                    0,
                    MSG_DONTWAIT,
                    ptr::null(),
                    0,
                )
            };

            if ret < 0 {
                match get_errno() {
                    ENOBUFS | EAGAIN | EBUSY | ENETDOWN => (),
                    _ => return Err(io::Error::last_os_error()),
                }
            }
        }

        Ok(cnt)
    }

    fn needs_wakeup(&self) -> bool {
        unsafe {
            if libbpf_sys::_xsk_ring_prod__needs_wakeup(self.inner.as_ref()) != 0 {
                true
            } else {
                false
            }
        }
    }
}
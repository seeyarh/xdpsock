# xdpsock

A Rust interface for the Linux AF_XDP address family.



[API documentation](https://docs.rs/xdpsock).

For more information please see the [networking docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
or a more [detailed overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).

This crate builds on the great work of [xsk-rs](https://github.com/DouglasGray/xsk-rs).
### Basic Usage

```
use xsk_rs::{
    socket::{BindFlags, SocketConfig, SocketConfigBuilder, XdpFlags},
    umem::{UmemConfig, UmemConfigBuilder},
    xsk::Xsk2,
};

// Configuration
let umem_config = UmemConfigBuilder::new()
    .frame_count(8192)
    .comp_queue_size(4096)
    .fill_queue_size(4096)
    .build()
    .unwrap();

let socket_config = SocketConfigBuilder::new()
    .tx_queue_size(4096)
    .rx_queue_size(4096)
    .bind_flags(BindFlags::XDP_COPY) // Check your driver to see if you can use ZERO_COPY
    .xdp_flags(XdpFlags::XDP_FLAGS_SKB_MODE) // Check your driver to see if you can use DRV_MODE
    .build()
    .unwrap();

let n_tx_frames = umem_config.frame_count() / 2;

let mut xsk = Xsk2::new(&ifname, 0, umem_config, socket_config, n_tx_frames as usize);

// Sending a packet
let pkt: Vec<u8> = vec![];
xsk.send(&pkt);

// Receiving a packet
let (recvd_pkt, len) = xsk.recv().expect("failed to recv");
```


### Examples

A couple can be found in the `examples` directory.

### Running tests / examples

The integration tests run using [veth-util-rs](https://github.com/seeyarh/veth-util-rs),
which creates a pair of virtual ethernet interfaces.  By default, root
permissions are required to create virtual ethernet interfaces. To avoid
running cargo under `root` it's best to first build the tests/examples and run
the binaries directly.


```
# tests
cargo build --tests
sudo run_all_tests.sh

# examples
cargo build --examples --release
sudo target/release/examples/hello_xdp
sudo target/release/examples/dev2_to_dev1 -- [FLAGS] [OPTIONS]
```

### Compatibility

Linux kernel version 5.11.0.

[Check your driver for XDP compatibility](https://github.com/iovisor/bcc/blob/master/docs/kernel-versions.md#xdp)

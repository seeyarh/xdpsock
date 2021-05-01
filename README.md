# xdpsock

A Rust interface for the Linux AF_XDP address family.



[API documentation](https://docs.rs/xdpsock).

For more information please see the [networking docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
or a more [detailed overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).

This crate builds on the great work of [xsk-rs](https://github.com/DouglasGray/xsk-rs).
### Basic Usage

```
use xdpsock::{
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

The example `dev2_to_dev1.rs` sends/receives udp packets using the specified
interface. To run this example:
```
# Compile
cargo build --release --examples

# Add the veth pair
sudo ip link add veth0 type veth peer name veth1

# Get the MAC addresses of the veth pair
ip link show
11: veth1@veth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc noqueue state UP mode DEFAULT group default qlen 1000
    link/ether 82:ff:40:35:17:a2 brd ff:ff:ff:ff:ff:ff
12: veth0@veth1: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc noqueue state UP mode DEFAULT group default qlen 1000
    link/ether 9e:4f:30:9e:e1:31 brd ff:ff:ff:ff:ff:ff

# Start the receiver
sudo ./target/release/examples/dev2_to_dev1 --n-pkts=1000000 --src-ip "192.168.69.1" --src-port 1234 --dest-ip "192.168.69.2" --dest-port 4321 --dev veth1 --dest-mac "82:ff:40:35:17:a2" --src-mac "9e:4f:30:9e:e1:31" rx

# In another terminal, start the sender
sudo ./target/release/examples/dev2_to_dev1 --n-pkts=1000000 --src-ip "192.168.69.1" --src-port 1234 --dest-ip "192.168.69.2" --dest-port 4321 --dev veth0 --dest-mac "82:ff:40:35:17:a2" --src-mac "9e:4f:30:9e:e1:31" tx
```

The example `synacker.rs` listens on the specified interface and responds to TCP
syn packets matching a given filter with a SYNACK.

```
sudo ./target/release/examples/synacker --src-ip "192.168.69.1" --dev "veth1"
```


### Running tests

The integration tests run using [veth-util-rs](https://github.com/seeyarh/veth-util-rs),
which creates a pair of virtual ethernet interfaces.  By default, root
permissions are required to create virtual ethernet interfaces. To avoid
running cargo under `root` it's best to first build the tests/examples and run
the binaries directly.


```
# tests
cargo build --tests
sudo run_all_tests.sh
```

### Compatibility

Linux kernel version 5.11.0.

[Check your driver for XDP compatibility](https://github.com/iovisor/bcc/blob/master/docs/kernel-versions.md#xdp)

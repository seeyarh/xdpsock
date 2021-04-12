mod util;

use std::{net::Ipv4Addr, num::NonZeroU32, str, thread};
use veth_util_rs::{add_veth_link, VethConfig, VethPair};
use xsk_rs::{FillQueue, FrameDesc, RxQueue, Socket, SocketConfig, TxQueue, Umem, UmemConfig};

// Put umem at bottom so drop order is correct
struct SocketState<'umem> {
    fill_q: FillQueue<'umem>,
    tx_q: TxQueue<'umem>,
    rx_q: RxQueue<'umem>,
    frame_descs: Vec<FrameDesc<'umem>>,
    umem: Umem<'umem>,
}

fn build_socket_and_umem<'a, 'umem>(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    if_name: &'a str,
    queue_id: u32,
) -> SocketState {
    let (mut umem, fill_q, _comp_q, frame_descs) = Umem::builder(umem_config)
        .create_mmap()
        .expect(format!("failed to create mmap area for {}", if_name).as_str())
        .create_umem()
        .expect(format!("failed to create umem for {}", if_name).as_str());

    let (tx_q, rx_q) = Socket::new(socket_config, &mut umem, if_name, queue_id)
        .expect(format!("failed to build socket for {}", if_name).as_str());

    SocketState {
        umem,
        fill_q,
        tx_q,
        rx_q,
        frame_descs,
    }
}

fn hello_xdp(veth_pair: &VethPair) {
    // Create umem and socket configs
    let umem_config = UmemConfig::default();
    let socket_config = SocketConfig::default();

    let mut dev1 = build_socket_and_umem(
        umem_config.clone(),
        socket_config.clone(),
        &veth_pair.dev1().ifname(),
        0,
    );

    let mut dev2 = build_socket_and_umem(umem_config, socket_config, &veth_pair.dev2().ifname(), 0);

    let mut dev1_frames = dev1.frame_descs;

    let mut dev2_frames = dev2.frame_descs;

    // Want to send some data from dev1 to dev2. So we need to:
    // 1. Make sure that dev2 can receive data by adding frames to its FillQueue
    // 2. Update the dev1's UMEM with the data we want to send and update
    //    the corresponding frame descriptor with the data's length
    // 3. Hand over the frame to the kernel for transmission
    // 4. Read from dev2

    // 1. Add frames to dev2's FillQueue
    assert_eq!(
        unsafe { dev2.fill_q.produce(&dev2_frames[..]) },
        dev2_frames.len()
    );

    // 2. Update dev1's UMEM with the data we want to send and update the frame desc
    let send_frame = &mut dev1_frames[0];
    let data = "Hello, world!".as_bytes();

    println!("sending: {:?}", str::from_utf8(&data).unwrap());

    // Copy the data to the frame
    unsafe { send_frame.write_to_umem_checked(&data).unwrap() };

    assert_eq!(send_frame.len(), data.len());

    // 3. Hand over the frame to the kernel for transmission
    assert_eq!(
        unsafe { dev1.tx_q.produce_and_wakeup(&dev1_frames[..1]).unwrap() },
        1
    );

    // 4. Read from dev2
    let packets_recvd = dev2
        .rx_q
        .poll_and_consume(&mut dev2_frames[..], 10)
        .unwrap();

    // Check that one of the packets we received matches what we expect.
    for recv_frame in dev2_frames.iter().take(packets_recvd) {
        let frame_ref = unsafe { recv_frame.read_from_umem_checked(recv_frame.len()).unwrap() };

        // Check lengths match
        if recv_frame.len() == data.len() {
            // Check contents match
            if frame_ref[..data.len()] == data[..] {
                println!(
                    "received: {:?}",
                    str::from_utf8(&frame_ref[..data.len()]).unwrap()
                );
                return;
            }
        }
    }

    panic!("no matching frames received")
}

fn main() {
    // We'll keep track of ctrl+c events but not let them kill the process
    // immediately as we may need to clean up the veth pair.
    let ctrl_c_events = util::ctrl_channel().unwrap();

    let veth_config = VethConfig::default();
    let veth_pair = add_veth_link(&veth_config).expect("failed to create veth pair");

    // Run example in separate thread so that if it panics we can clean up here
    let (example_done_tx, example_done_rx) = crossbeam_channel::bounded(1);

    let handle = thread::spawn(move || {
        hello_xdp(&veth_pair);
        let _ = example_done_tx.send(());
    });

    // Wait for either the example to finish or for a ctrl+c event to occur
    crossbeam_channel::select! {
        recv(example_done_rx) -> _ => {
            // Example done
            if let Err(e) = handle.join() {
                println!("error running example: {:?}", e);
            }
        },
        recv(ctrl_c_events) -> _ => {
            println!("SIGINT received, deleting veth pair and exiting");
        }
    }
}

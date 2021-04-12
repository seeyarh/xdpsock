use crossbeam_channel::{self, Receiver};
use etherparse::PacketBuilder;

pub fn ctrl_channel() -> Result<Receiver<()>, ctrlc::Error> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })?;

    Ok(rx)
}

fn generate_random_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}

// Generate an ETH frame w/ UDP as transport layer and payload size `payload_len`
pub fn generate_eth_frame(
    mac1: &[u8; 6],
    mac2: &[u8; 6],
    ip1: [u8; 4],
    ip2: [u8; 4],
    payload_len: usize,
) -> Vec<u8> {
    let builder = PacketBuilder::ethernet2(*mac1, *mac2)
        .ipv4(
            ip1, ip2, 20, // time to live
        )
        .udp(
            1234, // src port
            1234, // dst port
        );

    let payload = generate_random_bytes(payload_len);

    let mut result = Vec::<u8>::with_capacity(builder.size(payload.len()));

    builder.write(&mut result, &payload).unwrap();

    result
}

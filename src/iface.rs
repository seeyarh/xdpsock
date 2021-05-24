//! Utils for working with network interface counters

use std::collections::HashMap;
use std::fs::read_to_string;

/// Stats provided via /sys/class/net/IFACE/statistics/
#[derive(Debug, Clone)]
pub struct InterfaceStats {
    tx: InterfaceStatsTx,
    rx: InterfaceStatsRx,
}

impl InterfaceStats {
    pub fn new(if_name: &str) -> Self {
        let rx = InterfaceStatsRx::new(if_name).expect("failed to get rx stats");
        let tx = InterfaceStatsTx::new(if_name).expect("failed to get tx stats");
        Self { rx, tx }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InterfaceStatsRx {
    rx_bytes: u64,
    rx_compressed: u64,
    rx_crc_errors: u64,
    rx_dropped: u64,
    rx_errors: u64,
    rx_fifo_errors: u64,
    rx_frame_errors: u64,
    rx_length_errors: u64,
    rx_missed_errors: u64,
    rx_nohandler: u64,
    rx_over_errors: u64,
    rx_packets: u64,
}
const RX_FIELD_NAMES: &'static [&'static str] = &[
    "rx_bytes",
    "rx_compressed",
    "rx_crc_errors",
    "rx_dropped",
    "rx_errors",
    "rx_fifo_errors",
    "rx_frame_errors",
    "rx_length_errors",
    "rx_missed_errors",
    "rx_nohandler",
    "rx_over_errors",
    "rx_packets",
];

impl InterfaceStatsRx {
    pub fn new(if_name: &str) -> Result<Self, std::io::Error> {
        let mut map = HashMap::new();
        for field in RX_FIELD_NAMES.into_iter() {
            let path = format!("/sys/class/net/{}/statistics/{}", if_name, field);
            let contents = read_to_string(path)?;
            let field_value: u64 = contents.trim().parse().expect("failed to parse");
            map.insert((*field).into(), field_value);
        }

        let stats = Self::from_map(map).expect("stats missing field");
        Ok(stats)
    }

    fn from_map(map: HashMap<String, u64>) -> Option<Self> {
        let mut stats = Self::default();
        stats.rx_bytes = *map.get("rx_bytes")?;
        stats.rx_compressed = *map.get("rx_compressed")?;
        stats.rx_crc_errors = *map.get("rx_crc_errors")?;
        stats.rx_dropped = *map.get("rx_dropped")?;
        stats.rx_errors = *map.get("rx_errors")?;
        stats.rx_fifo_errors = *map.get("rx_fifo_errors")?;
        stats.rx_frame_errors = *map.get("rx_frame_errors")?;
        stats.rx_length_errors = *map.get("rx_length_errors")?;
        stats.rx_missed_errors = *map.get("rx_missed_errors")?;
        stats.rx_nohandler = *map.get("rx_nohandler")?;
        stats.rx_over_errors = *map.get("rx_over_errors")?;
        stats.rx_packets = *map.get("rx_packets")?;

        Some(stats)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InterfaceStatsTx {
    tx_aborted_errors: u64,
    tx_bytes: u64,
    tx_carrier_errors: u64,
    tx_compressed: u64,
    tx_dropped: u64,
    tx_errors: u64,
    tx_fifo_errors: u64,
    tx_heartbeat_errors: u64,
    tx_packets: u64,
    tx_window_errors: u64,
}

const TX_FIELD_NAMES: &'static [&'static str] = &[
    "tx_aborted_errors",
    "tx_bytes",
    "tx_carrier_errors",
    "tx_compressed",
    "tx_dropped",
    "tx_errors",
    "tx_fifo_errors",
    "tx_heartbeat_errors",
    "tx_packets",
    "tx_window_errors",
];

impl InterfaceStatsTx {
    pub fn new(if_name: &str) -> Result<Self, std::io::Error> {
        let mut map = HashMap::new();
        for field in TX_FIELD_NAMES.into_iter() {
            let path = format!("/sys/class/net/{}/statistics/{}", if_name, field);
            let contents = read_to_string(path)?;
            let field_value: u64 = contents.trim().parse().expect("failed to parse");
            map.insert((*field).into(), field_value);
        }

        let stats = Self::from_map(map).expect("stats missing field");
        Ok(stats)
    }

    fn from_map(map: HashMap<String, u64>) -> Option<Self> {
        let mut stats = Self::default();

        stats.tx_aborted_errors = *map.get("tx_aborted_errors")?;
        stats.tx_bytes = *map.get("tx_bytes")?;
        stats.tx_carrier_errors = *map.get("tx_carrier_errors")?;
        stats.tx_compressed = *map.get("tx_compressed")?;
        stats.tx_dropped = *map.get("tx_dropped")?;
        stats.tx_errors = *map.get("tx_errors")?;
        stats.tx_fifo_errors = *map.get("tx_fifo_errors")?;
        stats.tx_heartbeat_errors = *map.get("tx_heartbeat_errors")?;
        stats.tx_packets = *map.get("tx_packets")?;
        stats.tx_window_errors = *map.get("tx_window_errors")?;

        Some(stats)
    }
}

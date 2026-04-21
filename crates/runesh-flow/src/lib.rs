#![deny(unsafe_code)]
//! Network flow collector: NetFlow v5/v9, sFlow, IPFIX parsing.

pub mod netflow;

pub use netflow::{NetflowV5Header, ip_in_allowlist, listen_netflow_v5, parse_netflow_v5};

use std::collections::HashMap;
use std::net::Ipv4Addr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Default cap on the number of distinct aggregation keys held in memory.
pub const DEFAULT_MAX_ENTRIES: usize = 100_000;

/// A parsed network flow record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRecord {
    pub src_ip: Ipv4Addr,
    pub dst_ip: Ipv4Addr,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8,
    pub bytes: u64,
    pub packets: u64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    #[serde(default)]
    pub src_name: Option<String>,
    #[serde(default)]
    pub dst_name: Option<String>,
}

/// Flow protocol type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowProtocol {
    NetflowV5,
    NetflowV9,
    Ipfix,
    Sflow,
}

/// Top-N aggregation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopNEntry {
    pub key: String,
    pub bytes: u64,
    pub packets: u64,
    pub flow_count: usize,
}

/// Aggregate flows into top-N talkers.
pub fn top_talkers(flows: &[FlowRecord], n: usize) -> Vec<TopNEntry> {
    let mut by_src: std::collections::HashMap<Ipv4Addr, (u64, u64, usize)> =
        std::collections::HashMap::new();
    for f in flows {
        let e = by_src.entry(f.src_ip).or_default();
        e.0 += f.bytes;
        e.1 += f.packets;
        e.2 += 1;
    }
    let mut entries: Vec<TopNEntry> = by_src
        .into_iter()
        .map(|(ip, (bytes, packets, count))| TopNEntry {
            key: ip.to_string(),
            bytes,
            packets,
            flow_count: count,
        })
        .collect();
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    entries.truncate(n);
    entries
}

/// Aggregate flows into top-N destinations.
pub fn top_destinations(flows: &[FlowRecord], n: usize) -> Vec<TopNEntry> {
    let mut by_dst: std::collections::HashMap<Ipv4Addr, (u64, u64, usize)> =
        std::collections::HashMap::new();
    for f in flows {
        let e = by_dst.entry(f.dst_ip).or_default();
        e.0 += f.bytes;
        e.1 += f.packets;
        e.2 += 1;
    }
    let mut entries: Vec<TopNEntry> = by_dst
        .into_iter()
        .map(|(ip, (bytes, packets, count))| TopNEntry {
            key: ip.to_string(),
            bytes,
            packets,
            flow_count: count,
        })
        .collect();
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    entries.truncate(n);
    entries
}

/// Bounded running aggregator for flows keyed by source IP.
///
/// Prevents unbounded memory growth when ingesting flows over long
/// windows. When the entry count exceeds `max_entries` the map is
/// reset with a loud warning. Consumers should snapshot the counters
/// before a reset if they need long-term retention.
pub struct BoundedFlowAggregator {
    max_entries: usize,
    by_src: HashMap<Ipv4Addr, (u64, u64, usize)>,
    resets: u64,
}

impl BoundedFlowAggregator {
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries: if max_entries == 0 {
                DEFAULT_MAX_ENTRIES
            } else {
                max_entries
            },
            by_src: HashMap::new(),
            resets: 0,
        }
    }

    pub fn default_sized() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES)
    }

    /// Add a flow record to the aggregator.
    pub fn record(&mut self, flow: &FlowRecord) {
        if self.by_src.len() >= self.max_entries && !self.by_src.contains_key(&flow.src_ip) {
            self.resets = self.resets.saturating_add(1);
            tracing::warn!(
                max_entries = self.max_entries,
                resets = self.resets,
                "flow aggregator exceeded max_entries, resetting map"
            );
            self.by_src.clear();
        }
        let e = self.by_src.entry(flow.src_ip).or_default();
        e.0 = e.0.saturating_add(flow.bytes);
        e.1 = e.1.saturating_add(flow.packets);
        e.2 = e.2.saturating_add(1);
    }

    pub fn len(&self) -> usize {
        self.by_src.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_src.is_empty()
    }

    pub fn resets(&self) -> u64 {
        self.resets
    }

    /// Emit the current top-N talkers by bytes.
    pub fn top_n(&self, n: usize) -> Vec<TopNEntry> {
        let mut entries: Vec<TopNEntry> = self
            .by_src
            .iter()
            .map(|(ip, (bytes, packets, count))| TopNEntry {
                key: ip.to_string(),
                bytes: *bytes,
                packets: *packets,
                flow_count: *count,
            })
            .collect();
        entries.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        entries.truncate(n);
        entries
    }
}

/// Aggregate flows by protocol number.
pub fn by_protocol(flows: &[FlowRecord]) -> Vec<TopNEntry> {
    let mut by_proto: std::collections::HashMap<u8, (u64, u64, usize)> =
        std::collections::HashMap::new();
    for f in flows {
        let e = by_proto.entry(f.protocol).or_default();
        e.0 += f.bytes;
        e.1 += f.packets;
        e.2 += 1;
    }
    let mut entries: Vec<TopNEntry> = by_proto
        .into_iter()
        .map(|(proto, (bytes, packets, count))| {
            let name = match proto {
                6 => "TCP",
                17 => "UDP",
                1 => "ICMP",
                _ => "other",
            };
            TopNEntry {
                key: format!("{name} ({proto})"),
                bytes,
                packets,
                flow_count: count,
            }
        })
        .collect();
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_flows() -> Vec<FlowRecord> {
        let now = Utc::now();
        vec![
            FlowRecord {
                src_ip: "10.0.0.1".parse().unwrap(),
                dst_ip: "1.1.1.1".parse().unwrap(),
                src_port: 12345,
                dst_port: 443,
                protocol: 6,
                bytes: 5000,
                packets: 10,
                start_time: now,
                end_time: now,
                src_name: None,
                dst_name: None,
            },
            FlowRecord {
                src_ip: "10.0.0.1".parse().unwrap(),
                dst_ip: "8.8.8.8".parse().unwrap(),
                src_port: 12346,
                dst_port: 53,
                protocol: 17,
                bytes: 200,
                packets: 2,
                start_time: now,
                end_time: now,
                src_name: None,
                dst_name: None,
            },
            FlowRecord {
                src_ip: "10.0.0.2".parse().unwrap(),
                dst_ip: "1.1.1.1".parse().unwrap(),
                src_port: 54321,
                dst_port: 80,
                protocol: 6,
                bytes: 3000,
                packets: 5,
                start_time: now,
                end_time: now,
                src_name: None,
                dst_name: None,
            },
        ]
    }

    #[test]
    fn top_talkers_by_bytes() {
        let top = top_talkers(&sample_flows(), 10);
        assert_eq!(top[0].key, "10.0.0.1"); // 5200 bytes total
        assert_eq!(top[0].bytes, 5200);
    }

    #[test]
    fn top_destinations_by_bytes() {
        let top = top_destinations(&sample_flows(), 10);
        assert_eq!(top[0].key, "1.1.1.1"); // 8000 bytes
    }

    #[test]
    fn protocol_breakdown() {
        let protos = by_protocol(&sample_flows());
        assert!(protos.iter().any(|p| p.key.contains("TCP")));
        assert!(protos.iter().any(|p| p.key.contains("UDP")));
    }

    #[test]
    fn flow_serialization() {
        let f = &sample_flows()[0];
        let json = serde_json::to_string(f).unwrap();
        let parsed: FlowRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.dst_port, 443);
    }

    #[test]
    fn bounded_aggregator_enforces_cap() {
        let mut agg = BoundedFlowAggregator::new(2);
        let now = Utc::now();
        let make = |a, b, c, d| FlowRecord {
            src_ip: Ipv4Addr::new(a, b, c, d),
            dst_ip: "1.1.1.1".parse().unwrap(),
            src_port: 1,
            dst_port: 1,
            protocol: 6,
            bytes: 100,
            packets: 1,
            start_time: now,
            end_time: now,
            src_name: None,
            dst_name: None,
        };
        agg.record(&make(10, 0, 0, 1));
        agg.record(&make(10, 0, 0, 2));
        assert_eq!(agg.len(), 2);
        assert_eq!(agg.resets(), 0);
        // Third distinct src key should trigger a reset.
        agg.record(&make(10, 0, 0, 3));
        assert_eq!(agg.resets(), 1);
        assert_eq!(agg.len(), 1);
    }

    #[test]
    fn bounded_aggregator_merges_same_key() {
        let mut agg = BoundedFlowAggregator::new(10);
        let now = Utc::now();
        let mut f = FlowRecord {
            src_ip: "10.0.0.1".parse().unwrap(),
            dst_ip: "1.1.1.1".parse().unwrap(),
            src_port: 1,
            dst_port: 1,
            protocol: 6,
            bytes: 100,
            packets: 1,
            start_time: now,
            end_time: now,
            src_name: None,
            dst_name: None,
        };
        agg.record(&f);
        f.bytes = 200;
        agg.record(&f);
        let top = agg.top_n(5);
        assert_eq!(top[0].bytes, 300);
        assert_eq!(top[0].flow_count, 2);
    }
}

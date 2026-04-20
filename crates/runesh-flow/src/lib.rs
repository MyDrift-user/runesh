//! Network flow collector: NetFlow v5/v9, sFlow, IPFIX parsing.

use std::net::Ipv4Addr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
}

//! NetFlow v5 binary parser.
//!
//! Parses NetFlow v5 UDP datagrams into structured FlowRecords.
//! Header: 24 bytes. Each record: 48 bytes. Max 30 records per packet.
//! All fields are big-endian (network byte order).

use std::net::{IpAddr, Ipv4Addr};

use chrono::{DateTime, Utc};
use ipnet::IpNet;

use crate::FlowRecord;

/// NetFlow v5 packet header.
#[derive(Debug, Clone)]
pub struct NetflowV5Header {
    pub version: u16,
    pub count: u16,
    pub sys_uptime_ms: u32,
    pub unix_secs: u32,
    pub unix_nsecs: u32,
    pub flow_sequence: u32,
    pub engine_type: u8,
    pub engine_id: u8,
    pub sampling_interval: u16,
}

/// Parse a NetFlow v5 UDP datagram.
///
/// Returns the header and a vector of FlowRecords.
pub fn parse_netflow_v5(data: &[u8]) -> Result<(NetflowV5Header, Vec<FlowRecord>), String> {
    if data.len() < 24 {
        return Err(format!("packet too short: {} bytes (need 24)", data.len()));
    }

    let version = u16::from_be_bytes([data[0], data[1]]);
    if version != 5 {
        return Err(format!("not NetFlow v5: version {version}"));
    }

    let count = u16::from_be_bytes([data[2], data[3]]);
    if count > 30 {
        return Err(format!("invalid record count: {count} (max 30)"));
    }

    let expected_len = 24 + (count as usize) * 48;
    if data.len() < expected_len {
        return Err(format!(
            "packet truncated: {} bytes (need {expected_len})",
            data.len()
        ));
    }

    let unix_secs = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let unix_nsecs = u32::from_be_bytes([data[12], data[13], data[14], data[15]]);

    let header = NetflowV5Header {
        version,
        count,
        sys_uptime_ms: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
        unix_secs,
        unix_nsecs,
        flow_sequence: u32::from_be_bytes([data[16], data[17], data[18], data[19]]),
        engine_type: data[20],
        engine_id: data[21],
        sampling_interval: u16::from_be_bytes([data[22], data[23]]),
    };

    let base_time = DateTime::from_timestamp(unix_secs as i64, unix_nsecs).unwrap_or_else(Utc::now);

    let mut records = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let offset = 24 + i * 48;
        let r = &data[offset..offset + 48];

        let src_ip = Ipv4Addr::new(r[0], r[1], r[2], r[3]);
        let dst_ip = Ipv4Addr::new(r[4], r[5], r[6], r[7]);
        let packets = u32::from_be_bytes([r[16], r[17], r[18], r[19]]);
        let bytes = u32::from_be_bytes([r[20], r[21], r[22], r[23]]);
        let _first_uptime = u32::from_be_bytes([r[24], r[25], r[26], r[27]]);
        let _last_uptime = u32::from_be_bytes([r[28], r[29], r[30], r[31]]);
        let src_port = u16::from_be_bytes([r[32], r[33]]);
        let dst_port = u16::from_be_bytes([r[34], r[35]]);
        // r[36] is pad1
        let _tcp_flags = r[37];
        let protocol = r[38];
        let _tos = r[39];

        records.push(FlowRecord {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            protocol,
            bytes: bytes as u64,
            packets: packets as u64,
            start_time: base_time,
            end_time: base_time,
            src_name: None,
            dst_name: None,
        });
    }

    Ok((header, records))
}

/// Check whether `ip` is covered by any prefix in `allowed`.
pub fn ip_in_allowlist(ip: IpAddr, allowed: &[IpNet]) -> bool {
    allowed.iter().any(|net| net.contains(&ip))
}

/// Start a UDP listener for NetFlow v5 packets.
///
/// `allowed_sources` restricts which peer IPs are accepted. When `None`,
/// all exporters are accepted and a loud warning is emitted at startup.
/// When `Some(_)`, packets from non-listed peers are dropped silently.
///
/// Calls `on_flows` for each batch of parsed flow records.
pub async fn listen_netflow_v5<F>(
    bind_addr: &str,
    allowed_sources: Option<Vec<IpNet>>,
    mut on_flows: F,
) -> Result<(), std::io::Error>
where
    F: FnMut(NetflowV5Header, Vec<FlowRecord>),
{
    let socket = tokio::net::UdpSocket::bind(bind_addr).await?;
    match &allowed_sources {
        Some(list) => tracing::info!(
            %bind_addr,
            allowlist_entries = list.len(),
            "NetFlow v5 collector listening (allowlist enforced)"
        ),
        None => tracing::warn!(
            %bind_addr,
            "NetFlow v5 collector listening WITHOUT source allowlist; \
             accepting flows from any peer (set allowed_sources to restrict)"
        ),
    }

    let mut buf = [0u8; 2048]; // max packet: 24 + 30*48 = 1464 bytes
    loop {
        let (n, src) = socket.recv_from(&mut buf).await?;
        if let Some(list) = &allowed_sources
            && !ip_in_allowlist(src.ip(), list)
        {
            tracing::trace!(exporter = %src, "dropped NetFlow packet: source not in allowlist");
            continue;
        }
        match parse_netflow_v5(&buf[..n]) {
            Ok((header, records)) => {
                tracing::debug!(
                    exporter = %src,
                    count = records.len(),
                    sequence = header.flow_sequence,
                    "received NetFlow v5 packet"
                );
                on_flows(header, records);
            }
            Err(e) => {
                tracing::warn!(exporter = %src, error = %e, "invalid NetFlow v5 packet");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_v5_packet(count: u16) -> Vec<u8> {
        let mut pkt = vec![0u8; 24 + count as usize * 48];
        // Header
        pkt[0..2].copy_from_slice(&5u16.to_be_bytes()); // version
        pkt[2..4].copy_from_slice(&count.to_be_bytes()); // count
        pkt[4..8].copy_from_slice(&1000u32.to_be_bytes()); // uptime
        pkt[8..12].copy_from_slice(&1700000000u32.to_be_bytes()); // unix_secs
        pkt[16..20].copy_from_slice(&42u32.to_be_bytes()); // sequence

        for i in 0..count as usize {
            let offset = 24 + i * 48;
            // src IP: 10.0.0.{i+1}
            pkt[offset] = 10;
            pkt[offset + 3] = (i + 1) as u8;
            // dst IP: 1.1.1.1
            pkt[offset + 4] = 1;
            pkt[offset + 5] = 1;
            pkt[offset + 6] = 1;
            pkt[offset + 7] = 1;
            // packets: 100
            pkt[offset + 16..offset + 20].copy_from_slice(&100u32.to_be_bytes());
            // bytes: 50000
            pkt[offset + 20..offset + 24].copy_from_slice(&50000u32.to_be_bytes());
            // src port: 12345
            pkt[offset + 32..offset + 34].copy_from_slice(&12345u16.to_be_bytes());
            // dst port: 443
            pkt[offset + 34..offset + 36].copy_from_slice(&443u16.to_be_bytes());
            // protocol: TCP(6)
            pkt[offset + 38] = 6;
        }
        pkt
    }

    #[test]
    fn parse_single_record() {
        let pkt = build_v5_packet(1);
        let (header, records) = parse_netflow_v5(&pkt).unwrap();
        assert_eq!(header.version, 5);
        assert_eq!(header.count, 1);
        assert_eq!(header.flow_sequence, 42);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].src_ip, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(records[0].dst_ip, Ipv4Addr::new(1, 1, 1, 1));
        assert_eq!(records[0].src_port, 12345);
        assert_eq!(records[0].dst_port, 443);
        assert_eq!(records[0].protocol, 6);
        assert_eq!(records[0].bytes, 50000);
        assert_eq!(records[0].packets, 100);
    }

    #[test]
    fn parse_multiple_records() {
        let pkt = build_v5_packet(5);
        let (_header, records) = parse_netflow_v5(&pkt).unwrap();
        assert_eq!(records.len(), 5);
        assert_eq!(records[2].src_ip, Ipv4Addr::new(10, 0, 0, 3));
    }

    #[test]
    fn reject_wrong_version() {
        let mut pkt = build_v5_packet(1);
        pkt[0..2].copy_from_slice(&9u16.to_be_bytes());
        assert!(parse_netflow_v5(&pkt).is_err());
    }

    #[test]
    fn reject_truncated() {
        let pkt = build_v5_packet(1);
        assert!(parse_netflow_v5(&pkt[..40]).is_err()); // needs 72, got 40
    }

    #[test]
    fn reject_too_short() {
        assert!(parse_netflow_v5(&[0; 10]).is_err());
    }

    #[test]
    fn allowlist_matches_contained_ip() {
        let allow: Vec<IpNet> = vec![
            "10.0.0.0/8".parse().unwrap(),
            "192.168.1.0/24".parse().unwrap(),
        ];
        assert!(ip_in_allowlist("10.1.2.3".parse().unwrap(), &allow));
        assert!(ip_in_allowlist("192.168.1.7".parse().unwrap(), &allow));
        assert!(!ip_in_allowlist("172.16.0.1".parse().unwrap(), &allow));
    }

    #[test]
    fn allowlist_rejects_unlisted_ip() {
        let allow: Vec<IpNet> = vec!["127.0.0.1/32".parse().unwrap()];
        assert!(ip_in_allowlist("127.0.0.1".parse().unwrap(), &allow));
        assert!(!ip_in_allowlist("8.8.8.8".parse().unwrap(), &allow));
    }
}

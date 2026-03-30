//! Network interface information collection.

use sysinfo::Networks;

use crate::models::{IpAddress, IpVersion, NetworkInterface};

/// Collect network interface information.
pub fn collect_networks() -> Vec<NetworkInterface> {
    let networks = Networks::new_with_refreshed_list();

    networks
        .iter()
        .map(|(name, data)| {
            let mac = data.mac_address().to_string();
            let ip_addresses: Vec<IpAddress> = data
                .ip_networks()
                .iter()
                .map(|ip_net| {
                    let (address, version) = match ip_net.addr {
                        std::net::IpAddr::V4(v4) => (v4.to_string(), IpVersion::V4),
                        std::net::IpAddr::V6(v6) => (v6.to_string(), IpVersion::V6),
                    };
                    IpAddress {
                        address,
                        prefix_len: ip_net.prefix,
                        version,
                    }
                })
                .collect();

            NetworkInterface {
                name: name.to_string(),
                mac_address: mac,
                ip_addresses,
                is_up: true, // sysinfo only lists active interfaces
                bytes_received: data.total_received(),
                bytes_transmitted: data.total_transmitted(),
                packets_received: data.total_packets_received(),
                packets_transmitted: data.total_packets_transmitted(),
            }
        })
        .collect()
}

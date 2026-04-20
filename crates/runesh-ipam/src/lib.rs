//! IP address management (IPAM).
//!
//! Tracks prefixes, VLANs, individual IP assignments, and utilization.
//! NetBox-style data model for network planning.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::str::FromStr;

use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};

/// A managed IP prefix (subnet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefix {
    pub id: String,
    pub network: Ipv4Net,
    pub description: String,
    #[serde(default)]
    pub vlan_id: Option<u16>,
    #[serde(default)]
    pub site: Option<String>,
    pub status: PrefixStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrefixStatus {
    Active,
    Reserved,
    Deprecated,
}

/// A VLAN definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vlan {
    pub id: u16,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub site: Option<String>,
}

/// An individual IP address assignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpAssignment {
    pub address: Ipv4Addr,
    pub prefix_id: String,
    pub assigned_to: String,
    pub description: String,
    pub status: IpStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IpStatus {
    Active,
    Reserved,
    Deprecated,
    Dhcp,
}

/// Prefix utilization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixUtilization {
    pub prefix_id: String,
    pub network: String,
    pub total_hosts: u32,
    pub assigned: u32,
    pub available: u32,
    pub utilization_percent: f64,
}

/// IPAM store.
#[derive(Debug, Default)]
pub struct IpamStore {
    prefixes: HashMap<String, Prefix>,
    vlans: HashMap<u16, Vlan>,
    assignments: Vec<IpAssignment>,
}

impl IpamStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_prefix(&mut self, prefix: Prefix) {
        self.prefixes.insert(prefix.id.clone(), prefix);
    }

    pub fn add_vlan(&mut self, vlan: Vlan) {
        self.vlans.insert(vlan.id, vlan);
    }

    /// Allocate the next available IP in a prefix.
    pub fn allocate(
        &mut self,
        prefix_id: &str,
        assigned_to: &str,
        description: &str,
    ) -> Result<Ipv4Addr, IpamError> {
        let prefix = self
            .prefixes
            .get(prefix_id)
            .ok_or_else(|| IpamError::PrefixNotFound(prefix_id.into()))?;

        let network = prefix.network;
        let assigned_in_prefix: std::collections::HashSet<Ipv4Addr> = self
            .assignments
            .iter()
            .filter(|a| a.prefix_id == prefix_id)
            .map(|a| a.address)
            .collect();

        // Skip network address and broadcast
        for host in network.hosts() {
            if !assigned_in_prefix.contains(&host) {
                self.assignments.push(IpAssignment {
                    address: host,
                    prefix_id: prefix_id.into(),
                    assigned_to: assigned_to.into(),
                    description: description.into(),
                    status: IpStatus::Active,
                });
                return Ok(host);
            }
        }

        Err(IpamError::Exhausted(prefix_id.into()))
    }

    /// Assign a specific IP.
    pub fn assign_specific(
        &mut self,
        prefix_id: &str,
        ip: Ipv4Addr,
        assigned_to: &str,
        description: &str,
    ) -> Result<(), IpamError> {
        let prefix = self
            .prefixes
            .get(prefix_id)
            .ok_or_else(|| IpamError::PrefixNotFound(prefix_id.into()))?;

        if !prefix.network.contains(&ip) {
            return Err(IpamError::OutOfRange(ip.to_string()));
        }

        if self
            .assignments
            .iter()
            .any(|a| a.address == ip && a.prefix_id == prefix_id)
        {
            return Err(IpamError::AlreadyAssigned(ip.to_string()));
        }

        self.assignments.push(IpAssignment {
            address: ip,
            prefix_id: prefix_id.into(),
            assigned_to: assigned_to.into(),
            description: description.into(),
            status: IpStatus::Active,
        });
        Ok(())
    }

    /// Release an IP back to the pool.
    pub fn release(&mut self, ip: Ipv4Addr) -> bool {
        let before = self.assignments.len();
        self.assignments.retain(|a| a.address != ip);
        self.assignments.len() < before
    }

    /// Get utilization for a prefix.
    pub fn utilization(&self, prefix_id: &str) -> Option<PrefixUtilization> {
        let prefix = self.prefixes.get(prefix_id)?;
        let total = prefix.network.hosts().count() as u32;
        let assigned = self
            .assignments
            .iter()
            .filter(|a| a.prefix_id == prefix_id)
            .count() as u32;
        let available = total.saturating_sub(assigned);
        let util = if total == 0 {
            0.0
        } else {
            (assigned as f64 / total as f64) * 100.0
        };

        Some(PrefixUtilization {
            prefix_id: prefix_id.into(),
            network: prefix.network.to_string(),
            total_hosts: total,
            assigned,
            available,
            utilization_percent: util,
        })
    }

    /// Find which prefix an IP belongs to.
    pub fn find_prefix(&self, ip: Ipv4Addr) -> Option<&Prefix> {
        self.prefixes.values().find(|p| p.network.contains(&ip))
    }

    /// Get all assignments for a device/user.
    pub fn assignments_for(&self, assigned_to: &str) -> Vec<&IpAssignment> {
        self.assignments
            .iter()
            .filter(|a| a.assigned_to == assigned_to)
            .collect()
    }

    pub fn prefix_count(&self) -> usize {
        self.prefixes.len()
    }
    pub fn assignment_count(&self) -> usize {
        self.assignments.len()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IpamError {
    #[error("prefix not found: {0}")]
    PrefixNotFound(String),
    #[error("prefix exhausted: {0}")]
    Exhausted(String),
    #[error("IP out of range: {0}")]
    OutOfRange(String),
    #[error("IP already assigned: {0}")]
    AlreadyAssigned(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> IpamStore {
        let mut store = IpamStore::new();
        store.add_prefix(Prefix {
            id: "lan".into(),
            network: "192.168.1.0/24".parse().unwrap(),
            description: "Office LAN".into(),
            vlan_id: Some(10),
            site: Some("HQ".into()),
            status: PrefixStatus::Active,
        });
        store.add_vlan(Vlan {
            id: 10,
            name: "Office".into(),
            description: "Office network".into(),
            site: Some("HQ".into()),
        });
        store
    }

    #[test]
    fn allocate_sequential() {
        let mut store = setup();
        let ip1 = store.allocate("lan", "server-1", "Web server").unwrap();
        let ip2 = store.allocate("lan", "server-2", "DB server").unwrap();
        assert_eq!(ip1, Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(ip2, Ipv4Addr::new(192, 168, 1, 2));
    }

    #[test]
    fn assign_specific_ip() {
        let mut store = setup();
        store
            .assign_specific("lan", Ipv4Addr::new(192, 168, 1, 100), "printer", "Printer")
            .unwrap();
        assert_eq!(store.assignment_count(), 1);
    }

    #[test]
    fn duplicate_rejected() {
        let mut store = setup();
        store
            .assign_specific("lan", Ipv4Addr::new(192, 168, 1, 50), "a", "")
            .unwrap();
        assert!(
            store
                .assign_specific("lan", Ipv4Addr::new(192, 168, 1, 50), "b", "")
                .is_err()
        );
    }

    #[test]
    fn out_of_range_rejected() {
        let mut store = setup();
        assert!(
            store
                .assign_specific("lan", Ipv4Addr::new(10, 0, 0, 1), "x", "")
                .is_err()
        );
    }

    #[test]
    fn release_ip() {
        let mut store = setup();
        let ip = store.allocate("lan", "tmp", "Temp").unwrap();
        assert!(store.release(ip));
        assert_eq!(store.assignment_count(), 0);
    }

    #[test]
    fn utilization_tracking() {
        let mut store = setup();
        store.allocate("lan", "a", "").unwrap();
        store.allocate("lan", "b", "").unwrap();
        let util = store.utilization("lan").unwrap();
        assert_eq!(util.assigned, 2);
        assert_eq!(util.total_hosts, 254); // /24 = 254 usable
        assert!(util.utilization_percent < 1.0);
    }

    #[test]
    fn find_prefix_for_ip() {
        let store = setup();
        assert!(store.find_prefix(Ipv4Addr::new(192, 168, 1, 50)).is_some());
        assert!(store.find_prefix(Ipv4Addr::new(10, 0, 0, 1)).is_none());
    }

    #[test]
    fn assignments_for_device() {
        let mut store = setup();
        store.allocate("lan", "server-1", "").unwrap();
        store.allocate("lan", "server-1", "").unwrap();
        store.allocate("lan", "server-2", "").unwrap();
        assert_eq!(store.assignments_for("server-1").len(), 2);
    }
}

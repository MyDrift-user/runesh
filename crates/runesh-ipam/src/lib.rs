#![deny(unsafe_code)]
//! IP address management (IPAM).
//!
//! Tracks prefixes, VLANs, individual IP assignments, and utilization.
//! NetBox-style data model for network planning.
//!
//! # Concurrency
//!
//! A single [`IpamStore`] instance is safe to share across threads: all
//! mutating operations acquire an internal lock so allocate/release races
//! cannot produce duplicate assignments (TOCTOU safe).
//!
//! For multi-instance deployments (e.g. multiple processes / replicas),
//! external coordination (database, Redis, etcd) is required. That is out
//! of scope for this crate.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Mutex;

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

#[derive(Debug, Default)]
struct IpamState {
    prefixes: HashMap<String, Prefix>,
    vlans: HashMap<u16, Vlan>,
    assignments: Vec<IpAssignment>,
}

/// IPAM store.
///
/// Internally synchronized. All methods take `&self`; a single store is
/// safe to share across threads. See the crate-level docs for multi-process
/// guidance.
#[derive(Debug, Default)]
pub struct IpamStore {
    state: Mutex<IpamState>,
}

impl IpamStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_prefix(&self, prefix: Prefix) {
        let mut s = self.state.lock().expect("ipam mutex poisoned");
        s.prefixes.insert(prefix.id.clone(), prefix);
    }

    pub fn add_vlan(&self, vlan: Vlan) {
        let mut s = self.state.lock().expect("ipam mutex poisoned");
        s.vlans.insert(vlan.id, vlan);
    }

    /// Allocate the next available IP in a prefix.
    ///
    /// The lookup for a free address and the insert of the new assignment
    /// happen under the same lock, so concurrent callers cannot both be
    /// handed the same address.
    pub fn allocate(
        &self,
        prefix_id: &str,
        assigned_to: &str,
        description: &str,
    ) -> Result<Ipv4Addr, IpamError> {
        let mut s = self.state.lock().expect("ipam mutex poisoned");

        let network = s
            .prefixes
            .get(prefix_id)
            .ok_or_else(|| IpamError::PrefixNotFound(prefix_id.into()))?
            .network;

        let assigned_in_prefix: std::collections::HashSet<Ipv4Addr> = s
            .assignments
            .iter()
            .filter(|a| a.prefix_id == prefix_id)
            .map(|a| a.address)
            .collect();

        // Skip network address and broadcast
        for host in network.hosts() {
            if !assigned_in_prefix.contains(&host) {
                s.assignments.push(IpAssignment {
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
        &self,
        prefix_id: &str,
        ip: Ipv4Addr,
        assigned_to: &str,
        description: &str,
    ) -> Result<(), IpamError> {
        let mut s = self.state.lock().expect("ipam mutex poisoned");

        let prefix = s
            .prefixes
            .get(prefix_id)
            .ok_or_else(|| IpamError::PrefixNotFound(prefix_id.into()))?;

        if !prefix.network.contains(&ip) {
            return Err(IpamError::OutOfRange(ip.to_string()));
        }

        if s.assignments
            .iter()
            .any(|a| a.address == ip && a.prefix_id == prefix_id)
        {
            return Err(IpamError::AlreadyAssigned(ip.to_string()));
        }

        s.assignments.push(IpAssignment {
            address: ip,
            prefix_id: prefix_id.into(),
            assigned_to: assigned_to.into(),
            description: description.into(),
            status: IpStatus::Active,
        });
        Ok(())
    }

    /// Release a single `(prefix_id, ip)` assignment.
    ///
    /// Returns `true` when an assignment matching exactly that prefix and
    /// address was removed. Use [`IpamStore::release_all`] if you want the
    /// legacy "remove every assignment with this IP in any prefix" behavior.
    pub fn release(&self, prefix_id: &str, ip: Ipv4Addr) -> bool {
        let mut s = self.state.lock().expect("ipam mutex poisoned");
        let before = s.assignments.len();
        s.assignments
            .retain(|a| !(a.address == ip && a.prefix_id == prefix_id));
        s.assignments.len() < before
    }

    /// Release every assignment with the given IP across all prefixes.
    ///
    /// Use this only when you know you want the global behavior; prefer
    /// [`IpamStore::release`] for normal use.
    pub fn release_all(&self, ip: Ipv4Addr) -> bool {
        let mut s = self.state.lock().expect("ipam mutex poisoned");
        let before = s.assignments.len();
        s.assignments.retain(|a| a.address != ip);
        s.assignments.len() < before
    }

    /// Get utilization for a prefix.
    pub fn utilization(&self, prefix_id: &str) -> Option<PrefixUtilization> {
        let s = self.state.lock().expect("ipam mutex poisoned");
        let prefix = s.prefixes.get(prefix_id)?;
        let total = prefix.network.hosts().count() as u32;
        let assigned = s
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

    /// Find which prefix an IP belongs to (returns a clone).
    pub fn find_prefix(&self, ip: Ipv4Addr) -> Option<Prefix> {
        let s = self.state.lock().expect("ipam mutex poisoned");
        s.prefixes
            .values()
            .find(|p| p.network.contains(&ip))
            .cloned()
    }

    /// Get all assignments for a device/user (returns clones).
    pub fn assignments_for(&self, assigned_to: &str) -> Vec<IpAssignment> {
        let s = self.state.lock().expect("ipam mutex poisoned");
        s.assignments
            .iter()
            .filter(|a| a.assigned_to == assigned_to)
            .cloned()
            .collect()
    }

    pub fn prefix_count(&self) -> usize {
        self.state
            .lock()
            .expect("ipam mutex poisoned")
            .prefixes
            .len()
    }
    pub fn assignment_count(&self) -> usize {
        self.state
            .lock()
            .expect("ipam mutex poisoned")
            .assignments
            .len()
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
        let store = IpamStore::new();
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
        let store = setup();
        let ip1 = store.allocate("lan", "server-1", "Web server").unwrap();
        let ip2 = store.allocate("lan", "server-2", "DB server").unwrap();
        assert_eq!(ip1, Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(ip2, Ipv4Addr::new(192, 168, 1, 2));
    }

    #[test]
    fn assign_specific_ip() {
        let store = setup();
        store
            .assign_specific("lan", Ipv4Addr::new(192, 168, 1, 100), "printer", "Printer")
            .unwrap();
        assert_eq!(store.assignment_count(), 1);
    }

    #[test]
    fn duplicate_rejected() {
        let store = setup();
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
        let store = setup();
        assert!(
            store
                .assign_specific("lan", Ipv4Addr::new(10, 0, 0, 1), "x", "")
                .is_err()
        );
    }

    #[test]
    fn release_ip() {
        let store = setup();
        let ip = store.allocate("lan", "tmp", "Temp").unwrap();
        assert!(store.release("lan", ip));
        assert_eq!(store.assignment_count(), 0);
    }

    #[test]
    fn release_rejects_other_prefix() {
        let store = setup();
        store.add_prefix(Prefix {
            id: "dmz".into(),
            network: "10.0.0.0/24".parse().unwrap(),
            description: "DMZ".into(),
            vlan_id: None,
            site: None,
            status: PrefixStatus::Active,
        });

        let lan_ip = store.allocate("lan", "a", "").unwrap();
        // Wrong prefix id should not release the lan assignment.
        assert!(!store.release("dmz", lan_ip));
        assert_eq!(store.assignment_count(), 1);
        assert!(store.release("lan", lan_ip));
        assert_eq!(store.assignment_count(), 0);
    }

    #[test]
    fn release_all_across_prefixes() {
        let store = setup();
        store.add_prefix(Prefix {
            id: "lan2".into(),
            // Overlapping network on purpose to simulate dual assignment.
            network: "192.168.1.0/24".parse().unwrap(),
            description: "Alt LAN".into(),
            vlan_id: None,
            site: None,
            status: PrefixStatus::Active,
        });

        store
            .assign_specific("lan", Ipv4Addr::new(192, 168, 1, 5), "a", "")
            .unwrap();
        store
            .assign_specific("lan2", Ipv4Addr::new(192, 168, 1, 5), "b", "")
            .unwrap();
        assert_eq!(store.assignment_count(), 2);
        assert!(store.release_all(Ipv4Addr::new(192, 168, 1, 5)));
        assert_eq!(store.assignment_count(), 0);
    }

    #[test]
    fn utilization_tracking() {
        let store = setup();
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
        let store = setup();
        store.allocate("lan", "server-1", "").unwrap();
        store.allocate("lan", "server-1", "").unwrap();
        store.allocate("lan", "server-2", "").unwrap();
        assert_eq!(store.assignments_for("server-1").len(), 2);
    }

    #[test]
    fn concurrent_allocation_no_duplicates() {
        use std::sync::Arc;
        use std::thread;

        let store = Arc::new(setup());
        let n_threads = 16;
        let per_thread = 10;

        let mut handles = Vec::new();
        for _ in 0..n_threads {
            let s = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                let mut mine = Vec::new();
                for _ in 0..per_thread {
                    let ip = s.allocate("lan", "worker", "").unwrap();
                    mine.push(ip);
                }
                mine
            }));
        }

        let mut all = Vec::new();
        for h in handles {
            all.extend(h.join().unwrap());
        }

        let expected = n_threads * per_thread;
        assert_eq!(all.len(), expected);
        let unique: std::collections::HashSet<_> = all.into_iter().collect();
        assert_eq!(unique.len(), expected, "duplicate IPs handed out");
        assert_eq!(store.assignment_count(), expected);
    }
}

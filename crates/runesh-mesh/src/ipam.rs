//! Mesh IP address allocation.
//!
//! Each tenant gets a /22 block (1022 usable hosts) from the CGNAT range
//! (100.64.0.0/10, which spans 100.64.0.0 through 100.127.255.255).
//!
//! With /22 blocks, we support up to 4096 tenants.
//! Within each /22: offset 1 through 1022 are allocatable.

use std::collections::HashSet;
use std::net::Ipv4Addr;

use crate::MeshError;

/// CGNAT base: 100.64.0.0
const CGNAT_BASE: u32 = 0x6440_0000; // 100.64.0.0

/// Number of addresses per tenant (/22 = 1024 addresses).
const BLOCK_SIZE: u32 = 1024;

/// Maximum usable offset within a block (skip .0 network and last broadcast).
const MAX_OFFSET: u16 = 1022;

/// Maximum number of tenants: (100.127.255.255 - 100.64.0.0 + 1) / 1024 = 4096.
const MAX_TENANTS: u16 = 4096;

/// Manages mesh IP allocation for a single tenant.
#[derive(Debug, Clone)]
pub struct TenantIpPool {
    tenant_id: u16,
    /// Base address of this tenant's /22 block.
    base: u32,
    /// Currently allocated offsets within the block.
    allocated: HashSet<u16>,
    /// Next offset to try.
    next_offset: u16,
}

impl TenantIpPool {
    /// Create an IP pool for a tenant.
    ///
    /// Tenant IDs 0 through 4095 are valid.
    pub fn new(tenant_id: u16) -> Result<Self, MeshError> {
        if tenant_id >= MAX_TENANTS {
            return Err(MeshError::IpPoolExhausted(format!(
                "tenant_id {tenant_id} exceeds max {}",
                MAX_TENANTS - 1
            )));
        }
        let base = CGNAT_BASE + (tenant_id as u32) * BLOCK_SIZE;
        Ok(Self {
            tenant_id,
            base,
            allocated: HashSet::new(),
            next_offset: 1,
        })
    }

    /// Allocate the next available mesh IP.
    pub fn allocate(&mut self) -> Result<Ipv4Addr, MeshError> {
        let start = self.next_offset;
        let mut offset = start;

        loop {
            if offset <= MAX_OFFSET && !self.allocated.contains(&offset) {
                self.allocated.insert(offset);
                self.next_offset = if offset >= MAX_OFFSET { 1 } else { offset + 1 };
                return Ok(Ipv4Addr::from(self.base + offset as u32));
            }

            offset += 1;
            if offset > MAX_OFFSET {
                offset = 1;
            }
            if offset == start {
                return Err(MeshError::IpPoolExhausted(format!(
                    "tenant {}",
                    self.tenant_id
                )));
            }
        }
    }

    /// Allocate a specific IP (for restoring from persistent state).
    pub fn allocate_specific(&mut self, ip: Ipv4Addr) -> Result<(), MeshError> {
        let ip_u32 = u32::from(ip);
        if ip_u32 < self.base || ip_u32 >= self.base + BLOCK_SIZE {
            return Err(MeshError::IpPoolExhausted(format!(
                "{ip} not in tenant {} range",
                self.tenant_id
            )));
        }
        let offset = (ip_u32 - self.base) as u16;
        if !self.allocated.insert(offset) {
            return Err(MeshError::DuplicatePeer(format!("{ip} already allocated")));
        }
        Ok(())
    }

    /// Release an IP back to the pool.
    pub fn release(&mut self, ip: Ipv4Addr) {
        let ip_u32 = u32::from(ip);
        if ip_u32 >= self.base && ip_u32 < self.base + BLOCK_SIZE {
            let offset = (ip_u32 - self.base) as u16;
            self.allocated.remove(&offset);
        }
    }

    /// Get the base address of this tenant's block.
    pub fn subnet(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.base)
    }

    /// Number of allocated IPs.
    pub fn allocated_count(&self) -> usize {
        self.allocated.len()
    }

    /// Number of available IPs.
    pub fn available_count(&self) -> usize {
        MAX_OFFSET as usize - self.allocated.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_0_gets_correct_range() {
        let pool = TenantIpPool::new(0).unwrap();
        assert_eq!(pool.subnet(), Ipv4Addr::new(100, 64, 0, 0));
    }

    #[test]
    fn tenant_1_gets_next_range() {
        let pool = TenantIpPool::new(1).unwrap();
        // Tenant 1: 100.64.0.0 + 1024 = 100.64.4.0
        assert_eq!(pool.subnet(), Ipv4Addr::new(100, 64, 4, 0));
    }

    #[test]
    fn allocate_sequential() {
        let mut pool = TenantIpPool::new(0).unwrap();
        let ip1 = pool.allocate().unwrap();
        let ip2 = pool.allocate().unwrap();
        assert_eq!(ip1, Ipv4Addr::new(100, 64, 0, 1));
        assert_eq!(ip2, Ipv4Addr::new(100, 64, 0, 2));
    }

    #[test]
    fn release_and_reuse() {
        let mut pool = TenantIpPool::new(0).unwrap();
        let ip1 = pool.allocate().unwrap();
        let _ip2 = pool.allocate().unwrap();
        pool.release(ip1);
        assert_eq!(pool.allocated_count(), 1);
    }

    #[test]
    fn allocate_specific_ip() {
        let mut pool = TenantIpPool::new(0).unwrap();
        pool.allocate_specific(Ipv4Addr::new(100, 64, 0, 50))
            .unwrap();
        assert_eq!(pool.allocated_count(), 1);
    }

    #[test]
    fn duplicate_specific_rejected() {
        let mut pool = TenantIpPool::new(0).unwrap();
        pool.allocate_specific(Ipv4Addr::new(100, 64, 0, 50))
            .unwrap();
        assert!(
            pool.allocate_specific(Ipv4Addr::new(100, 64, 0, 50))
                .is_err()
        );
    }

    #[test]
    fn out_of_range_rejected() {
        let mut pool = TenantIpPool::new(0).unwrap();
        // Tenant 0 is 100.64.0.0 - 100.64.3.255, so 100.64.4.0 is tenant 1
        assert!(
            pool.allocate_specific(Ipv4Addr::new(100, 64, 4, 0))
                .is_err()
        );
    }

    #[test]
    fn max_tenant_id() {
        let pool = TenantIpPool::new(4095).unwrap();
        // 4095 * 1024 = 4,193,280 = 0x3FFC00
        // 0x6440_0000 + 0x003F_FC00 = 0x647F_FC00 = 100.127.252.0
        assert_eq!(pool.subnet(), Ipv4Addr::new(100, 127, 252, 0));
    }

    #[test]
    fn over_max_tenant_rejected() {
        assert!(TenantIpPool::new(4096).is_err());
    }

    #[test]
    fn available_count() {
        let mut pool = TenantIpPool::new(0).unwrap();
        assert_eq!(pool.available_count(), 1022);
        pool.allocate().unwrap();
        assert_eq!(pool.available_count(), 1021);
    }

    #[test]
    fn pool_exhaustion() {
        let mut pool = TenantIpPool::new(0).unwrap();
        for _ in 0..1022 {
            pool.allocate().unwrap();
        }
        assert!(pool.allocate().is_err());
    }
}

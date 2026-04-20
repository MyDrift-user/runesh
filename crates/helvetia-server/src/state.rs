//! Shared server state.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use runesh_acl::AclPolicy;
use runesh_coord::{MapBuilder, NoiseKeypair, PreAuthKey};
use runesh_mesh::TenantIpPool;
use runesh_proxy::ProxyConfig;

/// Shared application state for the coordination server.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    /// Server's Noise keypair for TS2021 handshake.
    noise_keypair: NoiseKeypair,
    /// Map builder (nodes, users, ACLs -> per-node MapResponses).
    map_builder: RwLock<MapBuilder>,
    /// Pre-auth keys indexed by key string.
    pre_auth_keys: RwLock<HashMap<String, PreAuthKey>>,
    /// Per-tenant IP pools.
    ip_pools: RwLock<HashMap<String, TenantIpPool>>,
    /// Proxy config (resource routing).
    proxy_config: RwLock<ProxyConfig>,
    /// Next node ID counter.
    next_node_id: std::sync::atomic::AtomicU64,
}

impl AppState {
    /// Create a new server state with the given ACL policy.
    pub fn new(acl: AclPolicy) -> Self {
        let noise_keypair = NoiseKeypair::generate().expect("failed to generate noise keypair");

        Self {
            inner: Arc::new(Inner {
                noise_keypair,
                map_builder: RwLock::new(MapBuilder::new(acl)),
                pre_auth_keys: RwLock::new(HashMap::new()),
                ip_pools: RwLock::new(HashMap::new()),
                proxy_config: RwLock::new(ProxyConfig::default()),
                next_node_id: std::sync::atomic::AtomicU64::new(1),
            }),
        }
    }

    /// Get the server's Noise public key.
    pub fn noise_public_key(&self) -> &[u8] {
        self.inner.noise_keypair.public_key()
    }

    /// Get the Noise keypair (for handshake).
    pub fn noise_keypair(&self) -> &NoiseKeypair {
        &self.inner.noise_keypair
    }

    /// Allocate the next node ID.
    pub fn next_node_id(&self) -> u64 {
        self.inner
            .next_node_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Get the map builder for reading/writing nodes.
    pub fn map_builder(&self) -> &RwLock<MapBuilder> {
        &self.inner.map_builder
    }

    /// Get the pre-auth key store.
    pub fn pre_auth_keys(&self) -> &RwLock<HashMap<String, PreAuthKey>> {
        &self.inner.pre_auth_keys
    }

    /// Get or create an IP pool for a tenant.
    pub async fn ip_pool(&self, tenant_id: &str) -> Result<TenantIpPool, String> {
        let mut pools = self.inner.ip_pools.write().await;
        if let Some(pool) = pools.get(tenant_id) {
            return Ok(pool.clone());
        }
        // Assign a tenant ID based on pool count
        let tenant_num = pools.len() as u16;
        let pool =
            TenantIpPool::new(tenant_num).map_err(|e| format!("failed to create IP pool: {e}"))?;
        pools.insert(tenant_id.to_string(), pool.clone());
        Ok(pool)
    }

    /// Update an IP pool after allocation.
    pub async fn update_ip_pool(&self, tenant_id: &str, pool: TenantIpPool) {
        self.inner
            .ip_pools
            .write()
            .await
            .insert(tenant_id.to_string(), pool);
    }

    /// Get the proxy config.
    pub fn proxy_config(&self) -> &RwLock<ProxyConfig> {
        &self.inner.proxy_config
    }
}

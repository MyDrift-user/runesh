//! Agent identity and enrollment state.
//!
//! An agent's identity consists of:
//! - Machine key: persistent keypair generated on first run
//! - Node key: WireGuard key for the mesh, can be rotated
//! - Enrollment state: whether this agent is registered with a controller

use serde::{Deserialize, Serialize};

use runesh_mesh::WgKeypair;

/// Persisted agent identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    /// Unique agent ID (assigned by controller on enrollment).
    #[serde(default)]
    pub agent_id: Option<u64>,

    /// Machine key (base64-encoded private key). Generated on first run, never changes.
    pub machine_key: String,

    /// Node key (base64-encoded private key). WireGuard key for the mesh.
    pub node_key: String,

    /// Assigned mesh IP.
    #[serde(default)]
    pub mesh_ip: Option<String>,

    /// Controller URL.
    pub controller_url: String,

    /// Enrollment state.
    pub state: EnrollmentState,

    /// Hostname at enrollment time.
    #[serde(default)]
    pub hostname: String,

    /// Tags assigned to this agent.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Enrollment state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentState {
    /// Not yet enrolled. Needs to contact controller.
    New,
    /// Enrollment request sent, waiting for approval.
    Pending,
    /// Enrolled and authorized.
    Enrolled,
    /// Was enrolled but authorization was revoked.
    Revoked,
}

impl AgentIdentity {
    /// Create a new identity for first-run enrollment.
    pub fn new(controller_url: &str) -> Self {
        let machine_kp = WgKeypair::generate();
        let node_kp = WgKeypair::generate();

        Self {
            agent_id: None,
            machine_key: machine_kp.private_base64(),
            node_key: node_kp.private_base64(),
            mesh_ip: None,
            controller_url: controller_url.to_string(),
            state: EnrollmentState::New,
            hostname: hostname(),
            tags: vec![],
        }
    }

    /// Whether this agent is enrolled and ready.
    pub fn is_enrolled(&self) -> bool {
        self.state == EnrollmentState::Enrolled && self.agent_id.is_some()
    }

    /// Get the machine public key (base64).
    pub fn machine_public_key(&self) -> Result<String, runesh_mesh::MeshError> {
        let kp = WgKeypair::from_private_base64(&self.machine_key)?;
        Ok(kp.public_base64())
    }

    /// Get the node public key (base64).
    pub fn node_public_key(&self) -> Result<String, runesh_mesh::MeshError> {
        let kp = WgKeypair::from_private_base64(&self.node_key)?;
        Ok(kp.public_base64())
    }

    /// Rotate the node key (generates a new WireGuard keypair).
    pub fn rotate_node_key(&mut self) {
        let kp = WgKeypair::generate();
        self.node_key = kp.private_base64();
    }
}

/// Get the system hostname.
fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_identity() {
        let id = AgentIdentity::new("https://controller.example.com");
        assert_eq!(id.state, EnrollmentState::New);
        assert!(id.agent_id.is_none());
        assert!(!id.is_enrolled());
        assert!(!id.machine_key.is_empty());
        assert!(!id.node_key.is_empty());
        assert_ne!(id.machine_key, id.node_key);
    }

    #[test]
    fn public_keys() {
        let id = AgentIdentity::new("https://controller.example.com");
        let mk = id.machine_public_key().unwrap();
        let nk = id.node_public_key().unwrap();
        assert_eq!(mk.len(), 44); // 32 bytes base64
        assert_eq!(nk.len(), 44);
        assert_ne!(mk, nk);
    }

    #[test]
    fn rotate_node_key() {
        let mut id = AgentIdentity::new("https://controller.example.com");
        let old_key = id.node_key.clone();
        id.rotate_node_key();
        assert_ne!(id.node_key, old_key);
        // Machine key unchanged
        let old_machine = id.machine_key.clone();
        id.rotate_node_key();
        assert_eq!(id.machine_key, old_machine);
    }

    #[test]
    fn enrollment_state() {
        let mut id = AgentIdentity::new("https://controller.example.com");
        assert!(!id.is_enrolled());

        id.state = EnrollmentState::Enrolled;
        assert!(!id.is_enrolled()); // still no agent_id

        id.agent_id = Some(42);
        assert!(id.is_enrolled());

        id.state = EnrollmentState::Revoked;
        assert!(!id.is_enrolled());
    }

    #[test]
    fn json_roundtrip() {
        let id = AgentIdentity::new("https://controller.example.com");
        let json = serde_json::to_string_pretty(&id).unwrap();
        let parsed: AgentIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.controller_url, id.controller_url);
        assert_eq!(parsed.machine_key, id.machine_key);
        assert_eq!(parsed.state, EnrollmentState::New);
    }
}

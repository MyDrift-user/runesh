//! Identity persistence.
//!
//! Saves and loads the agent identity to/from a JSON file on disk.
//! The identity file contains the machine key, node key, enrollment
//! state, and controller URL.

use std::path::{Path, PathBuf};

use runesh_agent::AgentIdentity;

/// Default identity file location per platform.
pub fn default_identity_path() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\ProgramData\runesh\agent\identity.json")
    } else {
        PathBuf::from("/etc/runesh/agent/identity.json")
    }
}

/// Save identity to disk.
pub fn save_identity(path: &Path, identity: &AgentIdentity) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create config dir: {e}"))?;
    }

    let json =
        serde_json::to_string_pretty(identity).map_err(|e| format!("serialization failed: {e}"))?;

    std::fs::write(path, &json).map_err(|e| format!("failed to write identity: {e}"))?;

    tracing::debug!(path = %path.display(), "identity saved");
    Ok(())
}

/// Load identity from disk. Returns None if file doesn't exist.
pub fn load_identity(path: &Path) -> Result<Option<AgentIdentity>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let json =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read identity: {e}"))?;

    let identity: AgentIdentity =
        serde_json::from_str(&json).map_err(|e| format!("invalid identity file: {e}"))?;

    tracing::debug!(path = %path.display(), "identity loaded");
    Ok(Some(identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("helvetia-test-persist");
        let path = dir.join("identity.json");

        let identity = AgentIdentity::new("https://ctrl.example.com");
        save_identity(&path, &identity).unwrap();

        let loaded = load_identity(&path).unwrap().unwrap();
        assert_eq!(loaded.controller_url, identity.controller_url);
        assert_eq!(loaded.machine_key, identity.machine_key);
        assert_eq!(loaded.node_key, identity.node_key);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let path = std::env::temp_dir().join("helvetia-test-nonexistent.json");
        let result = load_identity(&path).unwrap();
        assert!(result.is_none());
    }
}

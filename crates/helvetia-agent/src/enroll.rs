//! Enrollment with the Helvetia controller.

use runesh_agent::{AgentIdentity, EnrollmentState};
use runesh_coord::{RegisterRequest, RegisterResponse};

/// Enroll this agent with the controller.
///
/// Sends a registration request with the agent's keys and optional
/// pre-auth key. On success, updates the identity with the assigned
/// node ID and mesh IP.
pub async fn enroll(
    identity: &mut AgentIdentity,
    auth_key: Option<&str>,
) -> Result<RegisterResponse, String> {
    let node_pub = identity
        .node_public_key()
        .map_err(|e| format!("bad node key: {e}"))?;
    let machine_pub = identity
        .machine_public_key()
        .map_err(|e| format!("bad machine key: {e}"))?;

    let req = RegisterRequest {
        node_key: node_pub,
        machine_key: machine_pub,
        hostname: identity.hostname.clone(),
        os: std::env::consts::OS.to_string(),
        auth_key: auth_key.map(|s| s.to_string()),
        tags: identity.tags.clone(),
        endpoints: vec![],
    };

    let url = format!("{}/api/v1/register", identity.controller_url);
    tracing::info!(%url, hostname = %identity.hostname, "enrolling with controller");

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("enrollment failed: {status} {body}"));
    }

    let result: RegisterResponse = resp
        .json()
        .await
        .map_err(|e| format!("bad response: {e}"))?;

    if result.authorized {
        identity.agent_id = result.node_id;
        identity.mesh_ip = result.mesh_ip.clone();
        identity.state = EnrollmentState::Enrolled;
        tracing::info!(
            node_id = ?result.node_id,
            mesh_ip = ?result.mesh_ip,
            "enrolled successfully"
        );
    } else if let Some(url) = &result.auth_url {
        identity.state = EnrollmentState::Pending;
        tracing::warn!(%url, "interactive auth required");
    } else if let Some(err) = &result.error {
        tracing::error!(%err, "enrollment rejected");
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_builds_correctly() {
        let identity = AgentIdentity::new("https://ctrl.example.com");

        let node_pub = identity.node_public_key().unwrap();
        let machine_pub = identity.machine_public_key().unwrap();

        let req = RegisterRequest {
            node_key: node_pub.clone(),
            machine_key: machine_pub.clone(),
            hostname: identity.hostname.clone(),
            os: "linux".into(),
            auth_key: Some("tskey-test".into()),
            tags: vec!["tag:server".into()],
            endpoints: vec![],
        };

        assert_eq!(req.node_key.len(), 44);
        assert_eq!(req.machine_key.len(), 44);
        assert_ne!(req.node_key, req.machine_key);
    }
}

//! Heartbeat loop.
//!
//! Periodically contacts the controller to report status and
//! receive tasks. Also fetches updated peer maps.

use std::time::Duration;

use runesh_auth::AgentIdentity;
use runesh_jobs::{AgentTask, TaskQueue};

/// Heartbeat payload sent to the controller.
#[derive(Debug, serde::Serialize)]
pub struct HeartbeatPayload {
    pub node_id: u64,
    pub hostname: String,
    pub os: String,
    pub version: String,
    pub uptime_secs: u64,
    pub mesh_ip: Option<String>,
    pub pending_tasks: usize,
    pub completed_tasks: usize,
}

/// Response from the controller to a heartbeat.
#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
pub struct HeartbeatResponse {
    /// New tasks to execute.
    #[serde(default)]
    pub tasks: Vec<AgentTask>,
    /// Updated peer map (if changed since last heartbeat).
    #[serde(default)]
    pub peer_map_changed: bool,
    /// Server timestamp.
    #[serde(default)]
    pub server_time: Option<String>,
}

/// Run the heartbeat loop.
///
/// Sends a heartbeat every `interval` seconds and processes the response.
pub async fn run_heartbeat(
    identity: &AgentIdentity,
    task_queue: &mut TaskQueue,
    interval: Duration,
    start_time: std::time::Instant,
) {
    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/heartbeat", identity.controller_url);

    loop {
        let payload = HeartbeatPayload {
            node_id: identity.agent_id.unwrap_or(0),
            hostname: identity.hostname.clone(),
            os: std::env::consts::OS.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: start_time.elapsed().as_secs(),
            mesh_ip: identity.mesh_ip.clone(),
            pending_tasks: task_queue.pending_count(),
            completed_tasks: task_queue.completed_count(),
        };

        match client.post(&url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<HeartbeatResponse>().await {
                    Ok(hr) => {
                        for task in hr.tasks {
                            if task_queue.enqueue(task) {
                                tracing::info!("received new task from controller");
                            }
                        }
                        if hr.peer_map_changed {
                            tracing::info!("peer map updated");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "bad heartbeat response");
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), "heartbeat rejected");
            }
            Err(e) => {
                tracing::warn!(error = %e, "heartbeat failed, will retry");
            }
        }

        tokio::time::sleep(interval).await;
    }
}

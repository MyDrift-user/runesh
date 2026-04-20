mod enroll;
mod heartbeat;
mod persist;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use runesh_auth::AgentIdentity;
use runesh_jobs::TaskQueue;

/// Helvetia endpoint agent.
#[derive(Parser)]
#[command(name = "runesh-agentd", about = "RUNESH mesh agent daemon")]
struct Cli {
    /// Controller URL (e.g., https://ctrl.example.com:8080).
    #[arg(long, env = "RUNESH_CONTROLLER")]
    controller: Option<String>,

    /// Pre-authentication key for unattended enrollment.
    #[arg(long, env = "RUNESH_AUTH_KEY")]
    auth_key: Option<String>,

    /// Identity file path.
    #[arg(long, default_value_t = default_identity_path())]
    identity_file: String,

    /// Heartbeat interval in seconds.
    #[arg(long, default_value = "60")]
    heartbeat_interval: u64,

    /// Tags to apply to this node.
    #[arg(long, value_delimiter = ',')]
    tags: Vec<String>,

    /// Just enroll and exit (don't run the daemon loop).
    #[arg(long)]
    enroll_only: bool,

    /// Print the node's public keys and exit.
    #[arg(long)]
    show_keys: bool,
}

fn default_identity_path() -> String {
    persist::default_identity_path()
        .to_string_lossy()
        .to_string()
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let identity_path = PathBuf::from(&cli.identity_file);

    // Load or create identity
    let mut identity = match persist::load_identity(&identity_path) {
        Ok(Some(id)) => {
            tracing::info!("loaded existing identity");
            id
        }
        Ok(None) => {
            let controller = cli.controller.as_deref().unwrap_or_else(|| {
                tracing::error!("--controller is required for first enrollment");
                std::process::exit(1);
            });
            tracing::info!("creating new identity");
            let mut id = AgentIdentity::new(controller);
            id.tags = cli.tags.clone();
            id
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to load identity");
            std::process::exit(1);
        }
    };

    // Show keys mode
    if cli.show_keys {
        match (identity.machine_public_key(), identity.node_public_key()) {
            (Ok(mk), Ok(nk)) => {
                println!("Machine public key: {mk}");
                println!("Node public key:    {nk}");
                println!("Enrollment state:   {:?}", identity.state);
                if let Some(ip) = &identity.mesh_ip {
                    println!("Mesh IP:            {ip}");
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                tracing::error!(error = %e, "invalid keys in identity");
                std::process::exit(1);
            }
        }
        return;
    }

    // Enroll if not already enrolled
    if !identity.is_enrolled() {
        match enroll::enroll(&mut identity, cli.auth_key.as_deref()).await {
            Ok(resp) => {
                if resp.authorized {
                    if let Err(e) = persist::save_identity(&identity_path, &identity) {
                        tracing::error!(error = %e, "failed to save identity");
                        std::process::exit(1);
                    }
                } else if let Some(url) = &resp.auth_url {
                    tracing::info!(%url, "visit this URL to authorize the node");
                    if cli.enroll_only {
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "enrollment failed");
                std::process::exit(1);
            }
        }
    }

    if cli.enroll_only {
        tracing::info!("enrollment complete, exiting");
        return;
    }

    // Run the daemon
    tracing::info!(
        node_id = ?identity.agent_id,
        mesh_ip = ?identity.mesh_ip,
        "starting agent daemon"
    );

    let mut task_queue = TaskQueue::new();
    let start_time = std::time::Instant::now();
    let interval = Duration::from_secs(cli.heartbeat_interval);

    // Heartbeat loop runs until process is killed
    heartbeat::run_heartbeat(&identity, &mut task_queue, interval, start_time).await;
}

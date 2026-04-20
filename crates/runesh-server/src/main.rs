mod api;
mod state;

use base64::Engine;
use tracing_subscriber::EnvFilter;

use runesh_acl::AclPolicy;
use runesh_relay::{RelayConfig, RelayServer};

use crate::state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load ACL policy (default: allow all for bootstrap)
    let acl = AclPolicy::from_json(
        r#"{
        "acls": [{"action": "accept", "src": ["*"], "dst": ["*:*"]}]
    }"#,
    )
    .expect("failed to parse default ACL");

    let state = AppState::new(acl);

    // Log the server's Noise public key
    let pub_key = base64::engine::general_purpose::STANDARD.encode(state.noise_public_key());
    tracing::info!(key = %pub_key, "server noise public key");

    // Build the API router
    let app = api::router(state);

    // Start the HTTP API server
    let api_addr = std::env::var("RUNESH_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    tracing::info!(%api_addr, "starting RUNESH coordination server");

    let listener = tokio::net::TcpListener::bind(&api_addr)
        .await
        .expect("failed to bind API address");

    // Start DERP relay in background
    let relay_addr = std::env::var("RUNESH_RELAY_ADDR").unwrap_or_else(|_| "0.0.0.0:3340".into());
    tokio::spawn(async move {
        let relay = RelayServer::new(RelayConfig {
            bind_addr: relay_addr,
            ..Default::default()
        });
        if let Err(e) = relay.run().await {
            tracing::error!(error = %e, "DERP relay failed");
        }
    });

    axum::serve(listener, app).await.expect("server failed");
}

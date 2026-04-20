//! HTTP API routes for the coordination server.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;

use runesh_coord::{Node, RegisterRequest, RegisterResponse};

use crate::state::AppState;

/// Build the API router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/key", get(get_server_key))
        .route("/api/v1/register", post(register_node))
        .route("/api/v1/nodes", get(list_nodes))
        .route("/api/v1/map/{node_id}", get(get_map))
        .route("/api/v1/resources", get(list_resources))
        .route("/api/v1/resources", post(upsert_resource))
        .route("/health", get(health))
        .with_state(state)
}

/// GET /key - Returns the server's Noise public key (base64).
/// Tailscale clients call this to learn the server's key before handshake.
async fn get_server_key(State(state): State<AppState>) -> impl IntoResponse {
    let key = base64::engine::general_purpose::STANDARD.encode(state.noise_public_key());
    key
}

/// GET /health
async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

/// POST /api/v1/register - Register a new node.
async fn register_node(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, String)> {
    let tenant_id = "default"; // TODO: extract from auth context

    // Validate auth key if provided
    if let Some(auth_key) = &req.auth_key {
        let keys = state.pre_auth_keys().read().await;
        if let Some(pak) = keys.get(auth_key) {
            if pak.used && !pak.reusable {
                return Ok(Json(RegisterResponse {
                    authorized: false,
                    node_id: None,
                    mesh_ip: None,
                    error: Some("pre-auth key already used".into()),
                    auth_url: None,
                }));
            }
        } else {
            return Ok(Json(RegisterResponse {
                authorized: false,
                node_id: None,
                mesh_ip: None,
                error: Some("invalid pre-auth key".into()),
                auth_url: None,
            }));
        }
    }

    // Allocate mesh IP
    let mut pool = state
        .ip_pool(tenant_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let mesh_ip = pool
        .allocate()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.update_ip_pool(tenant_id, pool).await;

    // Create node
    let node_id = state.next_node_id();
    let node = Node {
        id: node_id,
        stable_id: format!("stable-{node_id}"),
        name: req.hostname.clone(),
        key: req.node_key.clone(),
        machine_key: req.machine_key.clone(),
        addresses: vec![mesh_ip.to_string()],
        allowed_ips: vec![format!("{mesh_ip}/32")],
        endpoints: req.endpoints.clone(),
        derp: None,
        hostname: req.hostname.clone(),
        os: req.os.clone(),
        tags: req.tags.clone(),
        online: true,
        last_seen: Some(chrono::Utc::now().to_rfc3339()),
        user: None,
        authorized: req.auth_key.is_some(), // auto-authorize if pre-auth key
        created: Some(chrono::Utc::now().to_rfc3339()),
        key_expiry: None,
    };

    // Register in map builder
    {
        let mut builder = state.map_builder().write().await;
        builder.upsert_node(node);
    }

    // Mark pre-auth key as used
    if let Some(auth_key) = &req.auth_key {
        let mut keys = state.pre_auth_keys().write().await;
        if let Some(pak) = keys.get_mut(auth_key) {
            pak.used = true;
        }
    }

    tracing::info!(
        node_id,
        hostname = %req.hostname,
        mesh_ip = %mesh_ip,
        "node registered"
    );

    Ok(Json(RegisterResponse {
        authorized: req.auth_key.is_some(),
        node_id: Some(node_id),
        mesh_ip: Some(mesh_ip.to_string()),
        error: None,
        auth_url: if req.auth_key.is_none() {
            Some("/auth/login".into())
        } else {
            None
        },
    }))
}

/// GET /api/v1/nodes - List all nodes.
async fn list_nodes(State(state): State<AppState>) -> impl IntoResponse {
    let builder = state.map_builder().read().await;
    let node_ids = builder.node_ids();

    // Build maps for each node to get node details
    let nodes: Vec<Node> = node_ids
        .iter()
        .filter_map(|id| builder.build_map(*id).and_then(|m| m.node))
        .collect();

    Json(nodes)
}

/// GET /api/v1/map/:node_id - Get the map response for a specific node.
async fn get_map(
    State(state): State<AppState>,
    axum::extract::Path(node_id): axum::extract::Path<u64>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let builder = state.map_builder().read().await;
    let map = builder
        .build_map(node_id)
        .ok_or((StatusCode::NOT_FOUND, format!("node {node_id} not found")))?;
    Ok(Json(map))
}

/// GET /api/v1/resources - List proxy resources.
async fn list_resources(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.proxy_config().read().await;
    let resources: Vec<&runesh_proxy::Resource> = config.resources.values().collect();
    Json(serde_json::to_value(resources).unwrap_or_default())
}

/// POST /api/v1/resources - Create or update a proxy resource.
async fn upsert_resource(
    State(state): State<AppState>,
    Json(resource): Json<runesh_proxy::Resource>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let hostname = resource.hostname.clone();
    let mut config = state.proxy_config().write().await;
    config.upsert(resource);

    tracing::info!(%hostname, "resource upserted");

    Ok((StatusCode::CREATED, Json(serde_json::json!({"ok": true}))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use runesh_acl::AclPolicy;
    use tower::util::ServiceExt;

    fn test_app() -> Router {
        let acl = AclPolicy::from_json(
            r#"{
            "acls": [{"action": "accept", "src": ["*"], "dst": ["*:*"]}]
        }"#,
        )
        .unwrap();
        let state = AppState::new(acl);
        router(state)
    }

    #[tokio::test]
    async fn health_check() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_key() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/key").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let key = String::from_utf8(body.to_vec()).unwrap();
        // Base64 of 32 bytes = 44 chars
        assert_eq!(key.len(), 44);
    }

    #[tokio::test]
    async fn register_with_auth_key() {
        let acl = AclPolicy::from_json(
            r#"{
            "acls": [{"action": "accept", "src": ["*"], "dst": ["*:*"]}]
        }"#,
        )
        .unwrap();
        let state = AppState::new(acl);

        // Add a pre-auth key
        {
            let mut keys = state.pre_auth_keys().write().await;
            keys.insert(
                "tskey-test-123".into(),
                runesh_coord::PreAuthKey {
                    key: "tskey-test-123".into(),
                    tenant_id: "default".into(),
                    user: "admin".into(),
                    reusable: false,
                    ephemeral: false,
                    acl_tags: vec![],
                    expiration: "2027-01-01T00:00:00Z".into(),
                    used: false,
                },
            );
        }

        let app = router(state);

        let req_body = serde_json::json!({
            "nodeKey": "dGVzdG5vZGVrZXkxMjM0NTY3ODkwMTIzNDU2Nzg5MDEy",
            "machineKey": "dGVzdG1hY2hpbmVrZXkxMjM0NTY3ODkwMTIzNDU2Nzg5",
            "hostname": "test-node",
            "os": "linux",
            "authKey": "tskey-test-123"
        });

        let resp = app
            .oneshot(
                Request::post("/api/v1/register")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let resp: RegisterResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.authorized);
        assert!(resp.node_id.is_some());
        assert!(resp.mesh_ip.is_some());
        assert!(resp.mesh_ip.unwrap().starts_with("100.64."));
    }

    #[tokio::test]
    async fn register_without_auth_key_returns_auth_url() {
        let app = test_app();

        let req_body = serde_json::json!({
            "nodeKey": "dGVzdG5vZGVrZXkxMjM0NTY3ODkwMTIzNDU2Nzg5MDEy",
            "machineKey": "dGVzdG1hY2hpbmVrZXkxMjM0NTY3ODkwMTIzNDU2Nzg5",
            "hostname": "test-node",
            "os": "linux"
        });

        let resp = app
            .oneshot(
                Request::post("/api/v1/register")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let resp: RegisterResponse = serde_json::from_slice(&body).unwrap();
        assert!(!resp.authorized);
        assert!(resp.auth_url.is_some());
    }

    #[tokio::test]
    async fn list_nodes_empty() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/api/v1/nodes").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();
        assert!(nodes.is_empty());
    }
}

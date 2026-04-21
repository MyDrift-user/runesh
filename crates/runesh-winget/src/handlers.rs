//! Axum HTTP handlers for the WinGet REST source protocol.
//!
//! Public endpoints:
//! - GET  /information             (server metadata)
//! - POST /manifestSearch          (search packages)
//! - GET  /packageManifests/{id}   (full package manifest)
//!
//! Admin endpoints (require `Authorization: Bearer <token>` matching the
//! provided [`AdminAuth`] implementation):
//! - POST   /admin/packages                  (upsert one package)
//! - POST   /admin/import                    (bulk import JSON)
//! - DELETE /admin/packages/{id}             (delete one package)
//!
//! Mount admin routes only when you have a real token. Call
//! [`winget_router_with_admin`] and pass a `SharedAdminAuth`; callers that
//! read the token from `WINGET_ADMIN_TOKEN` should panic at startup if it is
//! missing (see [`crate::admin_token_from_env`]).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use tokio::sync::RwLock;

use crate::auth::{AuthError, SharedAdminAuth};
use crate::manifest::{ManifestPolicy, Package, SearchRequest};
use crate::repo::WingetRepo;

/// Shared state for the winget API.
pub type WingetState = Arc<RwLock<WingetRepo>>;

/// Combined state used by admin routes.
#[derive(Clone)]
pub struct WingetAdminState {
    pub repo: WingetState,
    pub auth: SharedAdminAuth,
    pub policy: ManifestPolicy,
}

/// Build the public read-only router. No authentication, but callers should
/// rate-limit these endpoints at the edge.
pub fn winget_router(state: WingetState) -> Router {
    Router::new()
        .route("/information", get(get_information))
        .route("/manifestSearch", post(manifest_search))
        .route("/packageManifests/{id}", get(get_package_manifest))
        .with_state(state)
}

/// Build a router with both the public read endpoints and the authenticated
/// admin endpoints. Callers must supply a real `auth` implementation.
pub fn winget_router_with_admin(state: WingetState, admin: WingetAdminState) -> Router {
    let public = winget_router(state);
    let admin_routes = Router::new()
        .route("/admin/packages", post(upsert_package))
        .route("/admin/import", post(import_packages))
        .route("/admin/packages/{id}", delete(delete_package))
        .with_state(admin);
    public.merge(admin_routes)
}

/// GET /information
async fn get_information(State(state): State<WingetState>) -> impl IntoResponse {
    let repo = state.read().await;
    Json(serde_json::json!({
        "Data": {
            "SourceIdentifier": repo.source_identifier,
            "ServerSupportedVersions": ["1.4.0", "1.5.0"],
            "UnsupportedPackageMatchFields": [],
            "RequiredPackageMatchFields": [],
            "UnsupportedQueryParameters": [],
            "RequiredQueryParameters": []
        }
    }))
}

/// POST /manifestSearch
async fn manifest_search(
    State(state): State<WingetState>,
    Json(request): Json<SearchRequest>,
) -> impl IntoResponse {
    let repo = state.read().await;
    let results = repo.search(&request);

    if results.is_empty() {
        return (StatusCode::NO_CONTENT, Json(serde_json::Value::Null));
    }

    (StatusCode::OK, Json(serde_json::json!({ "Data": results })))
}

/// GET /packageManifests/{id}
async fn get_package_manifest(
    State(state): State<WingetState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let repo = state.read().await;

    match repo.get_package(&id) {
        Some(pkg) => (StatusCode::OK, Json(serde_json::json!({ "Data": pkg }))),
        None => (StatusCode::NO_CONTENT, Json(serde_json::Value::Null)),
    }
}

fn bearer_from(headers: &HeaderMap) -> Result<&str, AuthError> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or(AuthError::MissingToken)?
        .to_str()
        .map_err(|_| AuthError::MissingToken)?;
    auth.strip_prefix("Bearer ").ok_or(AuthError::MissingToken)
}

async fn require_admin(state: &WingetAdminState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let token = bearer_from(headers).map_err(|_| StatusCode::UNAUTHORIZED)?;
    state
        .auth
        .authenticate(token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

/// POST /admin/packages
async fn upsert_package(
    State(state): State<WingetAdminState>,
    headers: HeaderMap,
    Json(pkg): Json<Package>,
) -> Result<StatusCode, StatusCode> {
    require_admin(&state, &headers).await?;
    pkg.validate(state.policy)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut repo = state.repo.write().await;
    repo.upsert_package(pkg);
    Ok(StatusCode::CREATED)
}

/// POST /admin/import
async fn import_packages(
    State(state): State<WingetAdminState>,
    headers: HeaderMap,
    body: String,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    require_admin(&state, &headers).await?;
    let pkgs: Vec<Package> = serde_json::from_str(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    for p in &pkgs {
        p.validate(state.policy)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
    }
    let mut repo = state.repo.write().await;
    let mut count = 0;
    for p in pkgs {
        repo.upsert_package(p);
        count += 1;
    }
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "imported": count })),
    ))
}

/// DELETE /admin/packages/{id}
async fn delete_package(
    State(state): State<WingetAdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    require_admin(&state, &headers).await?;
    let mut repo = state.repo.write().await;
    match repo.remove_package(&id) {
        Some(_) => Ok(StatusCode::NO_CONTENT),
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::util::ServiceExt;

    use crate::auth::StaticTokenAuth;
    use crate::manifest::*;

    fn test_state() -> WingetState {
        let mut repo = WingetRepo::new("TestRepo");
        repo.upsert_package(crate::repo::tests_helper::sample_package(
            "Mozilla.Firefox",
            "Firefox",
            "125.0",
        ));
        repo.upsert_package(crate::repo::tests_helper::sample_package(
            "Google.Chrome",
            "Chrome",
            "124.0",
        ));
        Arc::new(RwLock::new(repo))
    }

    fn test_app() -> Router {
        winget_router(test_state())
    }

    fn admin_app() -> Router {
        let repo = test_state();
        let admin = WingetAdminState {
            repo: repo.clone(),
            auth: Arc::new(StaticTokenAuth::new("s3cret")),
            policy: ManifestPolicy { allow_http: false },
        };
        winget_router_with_admin(repo, admin)
    }

    #[tokio::test]
    async fn information_endpoint() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/information").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["Data"]["SourceIdentifier"], "TestRepo");
        assert!(
            !json["Data"]["ServerSupportedVersions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn search_finds_package() {
        let app = test_app();
        let body = serde_json::json!({
            "Query": { "KeyWord": "firefox", "MatchType": "Substring" }
        });
        let resp = app
            .oneshot(
                Request::post("/manifestSearch")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["Data"][0]["PackageIdentifier"], "Mozilla.Firefox");
    }

    #[tokio::test]
    async fn search_no_results() {
        let app = test_app();
        let body = serde_json::json!({
            "Query": { "KeyWord": "nonexistent", "MatchType": "Exact" }
        });
        let resp = app
            .oneshot(
                Request::post("/manifestSearch")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn get_manifest() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::get("/packageManifests/Mozilla.Firefox")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["Data"]["PackageIdentifier"], "Mozilla.Firefox");
    }

    #[tokio::test]
    async fn get_manifest_not_found() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::get("/packageManifests/NonExistent.App")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    fn sample_admin_package() -> Package {
        Package {
            package_identifier: "Corp.App".into(),
            versions: vec![PackageVersion {
                package_version: "1.0.0".into(),
                default_locale: DefaultLocale {
                    package_locale: "en-US".into(),
                    publisher: "Corp".into(),
                    package_name: "App".into(),
                    short_description: "app".into(),
                    publisher_url: None,
                    package_url: None,
                    license: None,
                    moniker: None,
                    tags: vec![],
                },
                installers: vec![Installer {
                    architecture: Architecture::X64,
                    installer_type: InstallerType::Msi,
                    installer_url: "https://dl.example.com/app-1.0.0.msi".into(),
                    installer_sha256: "a".repeat(64),
                    scope: None,
                    installer_switches: None,
                    product_code: None,
                }],
                locales: vec![],
            }],
        }
    }

    #[tokio::test]
    async fn admin_upsert_rejected_without_token() {
        let app = admin_app();
        let pkg = sample_admin_package();
        let resp = app
            .oneshot(
                Request::post("/admin/packages")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&pkg).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_upsert_rejects_bad_hash() {
        let app = admin_app();
        let mut pkg = sample_admin_package();
        pkg.versions[0].installers[0].installer_sha256 = "not-hex".into();
        let resp = app
            .oneshot(
                Request::post("/admin/packages")
                    .header("authorization", "Bearer s3cret")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&pkg).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn admin_upsert_accepts_valid() {
        let app = admin_app();
        let pkg = sample_admin_package();
        let resp = app
            .oneshot(
                Request::post("/admin/packages")
                    .header("authorization", "Bearer s3cret")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&pkg).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }
}

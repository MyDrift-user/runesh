//! Axum HTTP handlers for the WinGet REST source protocol.
//!
//! Implements the 3 endpoints that winget expects:
//! - GET  /information         (server metadata)
//! - POST /manifestSearch      (search packages)
//! - GET  /packageManifests/{id} (full package manifest)

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::sync::RwLock;

use crate::manifest::SearchRequest;
use crate::repo::WingetRepo;

/// Shared state for the winget API.
pub type WingetState = Arc<RwLock<WingetRepo>>;

/// Build the Axum router for the winget REST source.
///
/// Mount this at your desired base path (e.g., `/api`).
/// Register with: `winget source add --name MyRepo --arg https://host/api --type Microsoft.Rest`
pub fn winget_router(state: WingetState) -> Router {
    Router::new()
        .route("/information", get(get_information))
        .route("/manifestSearch", post(manifest_search))
        .route("/packageManifests/{id}", get(get_package_manifest))
        .with_state(state)
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::util::ServiceExt;

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
            json["Data"]["ServerSupportedVersions"]
                .as_array()
                .unwrap()
                .len()
                > 0
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
}

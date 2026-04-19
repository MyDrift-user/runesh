//! OpenAPI/Swagger UI setup for Axum.
//!
//! Provides helpers to configure utoipa's Swagger UI with sensible defaults
//! and optional toggling via environment variable.
//!
//! # Usage
//!
//! ```ignore
//! use runesh_core::openapi::{setup_swagger, SwaggerConfig};
//! use utoipa::OpenApi;
//!
//! #[derive(OpenApi)]
//! #[openapi(
//!     info(title = "My API", version = "0.1.0"),
//!     paths(list_users, get_user),
//!     components(schemas(User, CreateUser)),
//!     security(("bearer" = [])),
//! )]
//! struct ApiDoc;
//!
//! let app = Router::new()
//!     .route("/api/v1/users", get(list_users).post(create_user));
//!
//! // Conditionally add Swagger UI
//! let app = setup_swagger(app, ApiDoc::openapi(), SwaggerConfig::from_env());
//! ```

#[cfg(feature = "openapi")]
use axum::Router;

#[cfg(feature = "openapi")]
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

/// Configuration for Swagger UI.
#[cfg(feature = "openapi")]
pub struct SwaggerConfig {
    /// Whether Swagger UI is enabled (default: true in dev, false in prod).
    pub enabled: bool,
    /// Path to serve Swagger UI at (default: "/swagger-ui").
    pub ui_path: String,
    /// Path to serve the OpenAPI JSON spec at (default: "/api/openapi.json").
    pub spec_path: String,
}

#[cfg(feature = "openapi")]
impl SwaggerConfig {
    /// Load config from environment variables:
    /// - `SWAGGER_ENABLED` (default: "true")
    /// - `SWAGGER_PATH` (default: "/swagger-ui")
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("SWAGGER_ENABLED")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(true),
            ui_path: std::env::var("SWAGGER_PATH").unwrap_or_else(|_| "/swagger-ui".into()),
            spec_path: "/api/openapi.json".into(),
        }
    }

    /// Always enabled (for dev).
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ui_path: "/swagger-ui".into(),
            spec_path: "/api/openapi.json".into(),
        }
    }

    /// Always disabled (for prod).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::enabled()
        }
    }
}

/// Add Bearer token security scheme to an OpenAPI doc.
///
/// Call this on your `OpenApi` instance to add JWT Bearer auth:
/// ```ignore
/// let mut doc = ApiDoc::openapi();
/// add_bearer_security(&mut doc);
/// ```
#[cfg(feature = "openapi")]
pub fn add_bearer_security(doc: &mut utoipa::openapi::OpenApi) {
    let scheme = SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer));
    if let Some(ref mut components) = doc.components {
        components
            .security_schemes
            .insert("bearer".to_string(), scheme.into());
    } else {
        let mut components = utoipa::openapi::Components::new();
        components
            .security_schemes
            .insert("bearer".to_string(), scheme.into());
        doc.components = Some(components);
    }
}

/// Conditionally mount Swagger UI and OpenAPI spec on a router.
///
/// If `config.enabled` is false, returns the router unchanged.
/// Swagger UI is served at `config.ui_path` and the JSON spec at `config.spec_path`.
///
/// Security: In production, set `SWAGGER_ENABLED=false` to hide the API spec.
/// The spec endpoint is public (no auth required) so developers can use it,
/// but it should be disabled in production to avoid information disclosure.
#[cfg(feature = "openapi")]
pub fn setup_swagger<S: Clone + Send + Sync + 'static>(
    router: Router<S>,
    doc: utoipa::openapi::OpenApi,
    config: SwaggerConfig,
) -> Router<S> {
    if !config.enabled {
        tracing::info!("Swagger UI disabled");
        return router;
    }

    tracing::info!(
        path = %config.ui_path,
        spec = %config.spec_path,
        "Swagger UI enabled"
    );

    let swagger_ui = utoipa_swagger_ui::SwaggerUi::new(config.ui_path.clone())
        .url(config.spec_path.clone(), doc);

    router.merge(swagger_ui)
}

/// Re-export utoipa types for convenience.
#[cfg(feature = "openapi")]
pub mod prelude {
    pub use utoipa::{self, IntoParams, OpenApi, ToSchema};
}

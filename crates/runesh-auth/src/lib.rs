pub mod error;
pub mod jwks;
pub mod oidc;
pub mod token;
pub mod store;
pub mod session;

#[cfg(feature = "axum")]
pub mod axum_middleware;
#[cfg(feature = "axum")]
pub mod handlers;

pub use error::AuthError;
pub use jwks::OidcVerifier;
pub use oidc::{OidcProvider, OidcSession, OidcSessionStore, OidcUserInfo};
#[cfg(feature = "redis")]
pub use oidc::RedisOidcSessionStore;
pub use token::{Claims, TokenConfig};
pub use store::AuthStore;
pub use session::SessionConfig;

#[cfg(feature = "axum")]
pub use axum_middleware::{ApiKeyVerifier, ApiKeyVerifierExt, AuthExemptPaths, JwtSecret};

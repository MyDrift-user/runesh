pub mod error;
pub mod jwks;
pub mod oidc;
pub mod session;
pub mod store;
pub mod token;

#[cfg(feature = "axum")]
pub mod axum_middleware;
#[cfg(feature = "mesh")]
pub mod enrollment;
#[cfg(feature = "axum")]
pub mod handlers;

pub use error::AuthError;
pub use jwks::OidcVerifier;
#[cfg(feature = "redis")]
pub use oidc::RedisOidcSessionStore;
pub use oidc::{OidcProvider, OidcSession, OidcSessionStore, OidcUserInfo};
pub use session::SessionConfig;
pub use store::AuthStore;
pub use token::{Claims, TokenConfig};

#[cfg(feature = "mesh")]
pub use enrollment::{AgentIdentity, EnrollmentState};

#[cfg(feature = "axum")]
pub use axum_middleware::{ApiKeyVerifier, ApiKeyVerifierExt, AuthExemptPaths, JwtSecret};

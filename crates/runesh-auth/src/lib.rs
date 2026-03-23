pub mod error;
pub mod oidc;
pub mod token;
pub mod store;
pub mod session;

#[cfg(feature = "axum")]
pub mod axum_middleware;
#[cfg(feature = "axum")]
pub mod handlers;

pub use error::AuthError;
pub use oidc::{OidcProvider, OidcSession, OidcSessionStore, OidcUserInfo};
pub use token::{Claims, TokenConfig};
pub use store::AuthStore;
pub use session::SessionConfig;

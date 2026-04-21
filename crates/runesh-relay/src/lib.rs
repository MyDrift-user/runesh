#![deny(unsafe_code)]
pub mod error;
pub mod frame;
pub mod server;

pub use error::RelayError;
pub use frame::{Frame, FrameType};
pub use server::{
    AuthMode, CHALLENGE_LEN, RelayAuthConfig, RelayConfig, RelayServer, compute_challenge_response,
};

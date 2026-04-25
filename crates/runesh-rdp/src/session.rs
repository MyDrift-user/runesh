//! `RdpSession` — public surface for the RDP capture path.
//!
//! Status: v1 scaffold. The public API is locked so consumers
//! (rumi-agent's `kind = "rdp"` channel handler) can compile against
//! the typed [`crate::error::RdpError`] surface today. The actual
//! IronRDP-driven session loop lands in a dedicated follow-up where
//! we can iterate against a live Windows target — IronRDP's API
//! shifts between point releases (0.13 → 0.14 alone moved the
//! pixel-format module, the connector `Config` shape, and several
//! private struct fields) and writing the wiring blind has produced
//! a long string of compile-only no-tests results.
//!
//! Each method returns [`RdpError::NotImplemented`] for now. Rumi
//! consumers replace their `anyhow::bail!("rdp not yet wired")` with
//! `Err(RdpError::NotImplemented)` and surface a clean operator
//! message ("RDP support is rolling out; track runesh#XXX") instead
//! of a generic anyhow string.

use secrecy::SecretString;
use tokio::sync::mpsc;

use crate::error::RdpError;
use crate::input::InputEvent;
use crate::precondition;

pub use runesh_desktop::encode::VideoSample;

/// Connect parameters. Stable; the field set won't shrink before 1.0.
#[derive(Debug)]
pub struct RdpLogonParams {
    /// IPv4 / IPv6 / hostname. For local logon this is `127.0.0.1`.
    pub host: String,
    /// Default RDP port is 3389.
    pub port: u16,
    /// AD `user@domain.local` or local `user`.
    pub username: String,
    /// Password, kept in [`SecretString`] so logs can't accidentally
    /// dump it.
    pub password: SecretString,
    /// Optional NetBIOS / DNS domain. Prefer this over baking the
    /// domain into `username` when authenticating into AD; CredSSP's
    /// structured form behaves more predictably across server SKUs.
    pub domain: Option<String>,
    /// Requested desktop width.
    pub width: u32,
    /// Requested desktop height.
    pub height: u32,
    /// Frame rate target in fps. The session loop emits at most one
    /// encoded sample per `1 / fps_target` seconds.
    pub fps_target: u32,
    /// `true` skips TLS certificate validation. Required when
    /// connecting to `127.0.0.1` (Windows generates a self-signed
    /// certificate per machine for the Terminal Server endpoint).
    pub ignore_cert: bool,
}

/// Live RDP session.
pub struct RdpSession {
    samples_rx: mpsc::Receiver<Result<VideoSample, RdpError>>,
    input_tx: mpsc::Sender<InputEvent>,
    width: u32,
    height: u32,
}

impl RdpSession {
    /// Open the session.
    ///
    /// Returns [`RdpError::NotEnabled`] if the precondition check
    /// finds Remote Desktop disabled in the registry, otherwise
    /// [`RdpError::NotImplemented`] until the IronRDP wiring lands.
    pub async fn connect(params: RdpLogonParams) -> Result<Self, RdpError> {
        if !precondition::rdp_enabled()? {
            return Err(RdpError::NotEnabled);
        }
        // Burn the params so callers don't get an "unused field"
        // warning for fields the v1 surface doesn't yet hand off.
        let _ = (
            &params.host,
            params.port,
            &params.username,
            params.domain.as_deref(),
            params.width,
            params.height,
            params.fps_target,
            params.ignore_cert,
            params.password.clone(),
        );
        Err(RdpError::NotImplemented)
    }

    /// Pull the next encoded sample. Always `None` in v1 (no session
    /// is actually running); shaped this way so consumer code can be
    /// written against the final API today.
    pub async fn next_sample(&mut self) -> Option<Result<VideoSample, RdpError>> {
        self.samples_rx.recv().await
    }

    /// Forward an operator event. Returns
    /// [`RdpError::NotImplemented`] in v1.
    pub async fn send_input(&self, _evt: InputEvent) -> Result<(), RdpError> {
        let _ = &self.input_tx;
        Err(RdpError::NotImplemented)
    }

    /// Live desktop dimensions reported by the server (zeros in v1).
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Stop the background task and close the connection. No-op in v1.
    pub fn close(self) {}
}

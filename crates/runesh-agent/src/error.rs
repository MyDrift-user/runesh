//! Agent error types.

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("not enrolled: {0}")]
    NotEnrolled(String),

    #[error("enrollment failed: {0}")]
    EnrollmentFailed(String),

    #[error("heartbeat failed: {0}")]
    HeartbeatFailed(String),

    #[error("controller unreachable: {0}")]
    ControllerUnreachable(String),

    #[error("task execution failed: {0}")]
    TaskFailed(String),

    #[error("self-update failed: {0}")]
    UpdateFailed(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("mesh error: {0}")]
    Mesh(#[from] runesh_mesh::MeshError),
}

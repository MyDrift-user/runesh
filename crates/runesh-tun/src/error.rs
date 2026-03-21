#[derive(Debug, thiserror::Error)]
pub enum TunError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

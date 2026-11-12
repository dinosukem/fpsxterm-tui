use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("application configuration: {0}")]
    Config(#[from] std::io::Error),
    #[error("terminal: {0}")]
    Terminal(#[from] anyhow::Error),
}

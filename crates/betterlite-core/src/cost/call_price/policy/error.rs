use thiserror::Error;

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("betterlite has no executable adapter for the {0} provider")]
    UnavailableAdapter(String),
}

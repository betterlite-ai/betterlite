use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlacementError {
    #[error("complexity tier {tier} is not in the active tier set")]
    UnknownTier { tier: String },
    #[error("the request token count failed: {message}")]
    Count { message: String },
}

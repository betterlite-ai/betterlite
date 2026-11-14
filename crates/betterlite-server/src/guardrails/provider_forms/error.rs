use thiserror::Error;

#[derive(Debug, Error)]
pub enum FormError {
    #[error("max depth of {limit} exceeded while processing model fields")]
    TooDeep { limit: usize },
}

#[derive(Debug, thiserror::Error)]
pub enum ExpiryError {
    #[error("credential expiry is outside the representable time range")]
    OutOfRange,
}

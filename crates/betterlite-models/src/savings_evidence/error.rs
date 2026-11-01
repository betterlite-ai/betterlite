#[derive(Debug, thiserror::Error)]
pub enum SavingsEvidenceError {
    #[error("tokens saved total overflows a 64 bit counter")]
    SavedTokensOverflow,
    #[error("{field} must not be negative")]
    NegativeAmount { field: &'static str },
    #[error("amount must be a JSON number")]
    NonNumericAmount,
}

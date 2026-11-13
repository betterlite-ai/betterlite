use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BetaFilterError {
    #[error("the beta headers config entry for provider {provider} is not an object")]
    ProviderMapping { provider: String },
    #[error(
        "the beta headers config maps beta header {token} for provider {provider} to neither a string nor null"
    )]
    MappingValue { provider: String, token: String },
    #[error("{field} must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
}

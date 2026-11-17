use betterlite_logging::private_json::error::PrivateJsonError;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error(transparent)]
    Bounded(#[from] PrivateJsonError),
    #[error("the key management document is not a JSON object")]
    NotAnObject,
    #[error("the key management document has an invalid field")]
    Fields(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("the key management document could not be rebuilt")]
    Document(#[source] serde_json::Error),
    #[error("Field {field} should be a valid dictionary")]
    InvalidDictionary {
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("temp_budget_increase and temp_budget_expiry must be set together")]
    TemporaryBudgetPair,
    #[error("either key or key_alias must be provided")]
    MissingKeyIdentifier,
    #[error("key is required")]
    MissingResponseKey,
    #[error("update_fields must specify at least one field to update.")]
    EmptyPatch,
    #[error("Provide either `key_ids` or `all_keys_in_team=True`, not both.")]
    SelectionConflict,
    #[error("Must provide either `key_ids` (non-empty) or `all_keys_in_team=True`.")]
    SelectionMissing,
    #[error("{field} is required")]
    MissingField { field: &'static str },
    #[error("{field} must be greater than 0")]
    NotPositive { field: String },
    #[error("the policy request matches no known request body")]
    RequestUnion,
    #[error("the stored budget_limits text is not JSON")]
    StoredWindowsJson(#[source] serde_json::Error),
    #[error("the stored budget_limits value is not a list")]
    StoredWindowsNotAList,
    #[error("stored budget_limits entry {index} is not an object")]
    StoredWindowsUnsupported { index: usize },
}

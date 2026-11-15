use thiserror::Error;

#[derive(Debug, Error)]
pub enum HookInitError {
    #[error("{vendor}: {setting} is required")]
    Required {
        vendor: &'static str,
        setting: &'static str,
    },
    #[error(
        "Microsoft Agent 365: client secret is required. Set client_secret, api_key, or AGENT365_CLIENT_SECRET"
    )]
    Agent365ClientSecretRequired,
    #[error("Microsoft Purview: client_secret (or api_key) is required")]
    PurviewClientSecretRequired,
    #[error("Azure Content Safety: {part} is not a valid guardrail")]
    AzureGuardrailNotValid { part: String },
    #[error("{vendor}: {field} has an invalid value")]
    InvalidField {
        vendor: &'static str,
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{vendor}: the guardrail parameters could not be read")]
    UnreadableParams {
        vendor: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

mod tests;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RealtimeError {
    #[error("api_base is required for Azure OpenAI calls")]
    MissingApiBase,
    #[error("api_version is required for Azure OpenAI calls")]
    MissingApiVersion,
    #[error(
        "Missing Azure credentials for the realtime endpoint. Set an api_key, or configure Azure AD auth (azure_ad_token, tenant_id/client_id/client_secret, or a managed identity)"
    )]
    MissingCredentials,
}

use betterlite_azure::credentials::selection::AzureTokenError;
use betterlite_azure::credentials::source::SourceFailure;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CredentialsError {
    #[error(
        "Missing Azure AI credentials - set an API key (`api_key` or {env_var}), or Entra ID / OAuth credentials (`tenant_id` + `client_id` + `client_secret`, `azure_ad_token`, an OIDC token, or a managed identity with `litellm.enable_azure_ad_token_refresh = True`)"
    )]
    MissingCredentials { env_var: String },
    #[error("Azure AI agent Entra ID credentials did not resolve to a token. {help}")]
    AgentTokenUnresolved { help: &'static str },
    #[error("litellm_params.{field} must be a string")]
    InvalidParam { field: &'static str },
    #[error("Azure AD token must be a string, got {type_name}")]
    TokenNotString { type_name: String },
    #[error("Failed to get Azure AD token: {message}")]
    ProviderFailed { message: String },
    #[error(transparent)]
    Token(#[from] AzureTokenError),
    #[error(transparent)]
    Source(#[from] SourceFailure),
    #[error(transparent)]
    Secret(#[from] SecretLookupError),
}

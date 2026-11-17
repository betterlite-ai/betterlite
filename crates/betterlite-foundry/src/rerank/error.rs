use betterlite_http::user_url::error::InvalidUrl;
use betterlite_openai::cohere_rerank::CohereRerankError;
use betterlite_openai::openai_base_config::forwarding::BaseConfigError;
use thiserror::Error;

use crate::credentials::error::CredentialsError;

#[derive(Debug, Error)]
pub enum FoundryRerankError {
    #[error(
        "Azure AI API Base is required. api_base=None. Set in call or via `AZURE_AI_API_BASE` env var."
    )]
    MissingApiBase,
    #[error(
        "Azure AI API Base must be an absolute URL including scheme (e.g. 'https://<resource>.services.ai.azure.com'). Got api_base={rendered}."
    )]
    RelativeApiBase { rendered: String },
    #[error("api_base is not a valid URL")]
    InvalidUrl(#[source] InvalidUrl),
    #[error("the api_base path cannot be carried into the rerank endpoint")]
    UnwritablePath,
    #[error("api_base cannot take the rerank endpoint path")]
    Compose(#[source] BaseConfigError),
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
    #[error(transparent)]
    Cohere(#[from] CohereRerankError),
}

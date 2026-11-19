use betterlite_openai::assistants::error::AssistantsError;
use thiserror::Error;
use url::ParseError;

use crate::endpoints::initialize::EndpointError;

#[derive(Debug, Error)]
pub enum DefinitionsError {
    #[error(transparent)]
    Endpoint(#[from] EndpointError),
    #[error(transparent)]
    Assistants(#[from] AssistantsError),
    #[error("the Azure endpoint is not a URL: {source}")]
    BaseUrl {
        #[source]
        source: ParseError,
    },
    #[error(
        "Missing credentials. Please pass one of `api_key`, `azure_ad_token`, `azure_ad_token_provider`, or the `AZURE_OPENAI_API_KEY` or `AZURE_OPENAI_AD_TOKEN` environment variables."
    )]
    MissingCredentials,
    #[error(
        "Must provide one of the `base_url` or `azure_endpoint` arguments, or the `AZURE_OPENAI_ENDPOINT` environment variable"
    )]
    MissingEndpoint,
    #[error("Expected the azure_ad_token_provider argument to return a string")]
    ProviderReply,
    #[error("the azure_ad_token_provider raised {class}: {message}")]
    Provider { class: String, message: String },
    #[error("the timeout {seconds} seconds is not a duration")]
    Timeout { seconds: f64 },
}

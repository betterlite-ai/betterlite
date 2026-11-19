use betterlite_openai::files::error::FilesError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::endpoints::initialize::EndpointError;

#[derive(Debug, Error)]
pub enum AzureFilesError {
    #[error(
        "AzureOpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment."
    )]
    ClientMissing,
    #[error(
        "AzureOpenAI client is not an instance of AsyncAzureOpenAI. Make sure you passed an AsyncAzureOpenAI client."
    )]
    NotAsyncClient,
    #[error(
        "Missing credentials. Please pass one of `api_key`, `azure_ad_token`, `azure_ad_token_provider`, or the `AZURE_OPENAI_API_KEY` or `AZURE_OPENAI_AD_TOKEN` environment variables."
    )]
    MissingCredentials,
    #[error("Unable to handle auth")]
    UnableToHandleAuth,
    #[error(
        "Expected `azure_ad_token_provider` argument to return a string but it returned {returned}"
    )]
    TokenNotString { returned: String },
    #[error("{message}")]
    TokenProvider { class: String, message: String },
    #[error(
        "Must provide one of the `base_url` or `azure_endpoint` arguments, or the `AZURE_OPENAI_ENDPOINT` environment variable"
    )]
    MissingBaseUrl,
    #[error("the Azure file client base URL is not a URL")]
    BaseUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("the Azure file API key is not a header value")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("`timeout` must be a positive number of seconds but was {seconds}")]
    Timeout { seconds: f64 },
    #[error(transparent)]
    Endpoint {
        #[from]
        source: EndpointError,
    },
    #[error(transparent)]
    Files {
        #[from]
        source: FilesError,
    },
}

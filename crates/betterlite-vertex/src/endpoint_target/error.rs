use regex::Error as RegexError;
use url::ParseError;

use crate::model_urls::error::VertexUrlError;

#[derive(Debug, thiserror::Error)]
pub enum EndpointTargetError {
    #[error("vertex URL: {0}")]
    VertexUrl(#[from] VertexUrlError),
    #[error("the configured base URL cannot be parsed: {source}")]
    BaseUrl {
        #[source]
        source: ParseError,
    },
    #[error("the configured base URL must use http or https")]
    BaseScheme,
    #[error("the default URL cannot be parsed: {source}")]
    DefaultUrl {
        #[source]
        source: ParseError,
    },
    #[error("the passthrough route pattern cannot be compiled: {source}")]
    Pattern {
        #[source]
        source: RegexError,
    },
    #[error("a PSC endpoint needs vertex_project, vertex_location and model")]
    PscInputs,
    #[error("a custom Gemini base needs a model")]
    GeminiModel,
    #[error("a custom Gemini base needs a Gemini API key")]
    GeminiApiKey,
    #[error("the requested route must be a path without a query or fragment")]
    Route,
    #[error("vertex_project is required")]
    Project,
    #[error("vertex_location is required")]
    Location,
}

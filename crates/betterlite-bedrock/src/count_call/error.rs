use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;
use url::ParseError;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::signing::error::SigningError;

use crate::bedrock_settings::HeaderError;
use crate::count_tokens::error::CountTokensError;

#[derive(Debug, Error)]
pub enum CountCallError {
    #[error(transparent)]
    Request(#[from] CountTokensError),
    #[error("the AWS region could not be resolved")]
    Region(#[from] AwsEndpointError),
    #[error("the litellm_params field {field} has the wrong type")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the CountTokens request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the CountTokens endpoint {endpoint:?} is not a URL")]
    Endpoint {
        endpoint: String,
        #[source]
        source: ParseError,
    },
    #[error("the {name} header name is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not valid")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the CountTokens request could not be signed")]
    Signing(#[from] SigningError),
    #[error("the CountTokens request could not be sent")]
    Transport(#[source] TransportError),
    #[error("the CountTokens response could not be decoded")]
    Decode(#[source] serde_json::Error),
    #[error("the CountTokens response is not a JSON object")]
    NotAnObject,
    #[error("input_tokens is not an integer")]
    InputTokens,
    #[error("the error record header cannot be carried")]
    ErrorHeader {
        #[source]
        source: HeaderError,
    },
}

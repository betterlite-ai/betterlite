use std::convert::Infallible;
use std::error::Error as StdError;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use http::{HeaderMap, StatusCode};
use thiserror::Error;

use crate::agentcore_stream::adapter::header_pairs;
use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};
use crate::converse_response::transform::TransformError;

// Invoke's make_call turns every opening failure into a BedrockError; the Converse sync call only does that for a non-200 reply.
#[derive(Debug, Error)]
pub enum HttpStreamError<CounterFailure = Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error("the Bedrock stream request failed")]
    Transport(#[source] TransportError),
    #[error("the api_base is not a URL")]
    Url(#[source] url::ParseError),
    #[error("the Bedrock stream body could not be read")]
    Body(#[source] HttpError),
    #[error("the reply could not be projected")]
    Transform(#[source] TransformError<CounterFailure>),
    #[error(transparent)]
    Header(#[from] HeaderError),
}

// What LiteLLM's catch-all raises: the caught exception's text becomes the message and no source survives.
pub(crate) fn raised<CounterFailure>(
    status: u16,
    message: String,
) -> HttpStreamError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let input = ErrorInput {
        status_code: status,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(error) => HttpStreamError::Provider {
            status,
            error: Box::new(error),
        },
        Err(source) => HttpStreamError::Header(source),
    }
}

// A converter that raised a BedrockError keeps it; every other conversion failure is a bare Python exception.
pub(crate) fn transform_failure(error: TransformError<Infallible>) -> HttpStreamError<Infallible> {
    match error {
        TransformError::Conversion { status, error } => HttpStreamError::Provider { status, error },
        other => HttpStreamError::Transform(other),
    }
}

pub(crate) fn status_error(
    status: u16,
    message: String,
    headers: &HeaderMap,
    url: &str,
    body: ResponseBody,
) -> Result<BedrockError, HeaderError> {
    let pairs = header_pairs(headers);
    let reply = ErrorReply {
        response: ErrorResponse {
            status_code: status,
            request: ErrorRequest {
                method: "POST".to_owned(),
                url: url.to_owned(),
            },
        },
        headers: pairs.clone(),
        body,
    };
    BedrockError::try_from(ErrorInput {
        status_code: status,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(pairs)),
        response: Some(reply),
    })
}

// httpx reports the reason line of the status it received, and an unknown code carries none.
pub(crate) fn reason_phrase(status: u16) -> String {
    StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or_default()
        .to_owned()
}

use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http::header::CONTENT_TYPE;
use reqwest::{Body, Request, Response};
use thiserror::Error;
use uuid::Uuid;

use crate::openai_mock_transport::{DecodeMockRequestError, handle_mock_request};

#[derive(Debug, Error)]
pub enum MockError {
    #[error("the request body is not a JSON object: {source}")]
    Request {
        #[source]
        source: DecodeMockRequestError,
    },
    #[error("the mocked answer could not be written as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the mocked answer could not be built")]
    Response(#[source] http::Error),
}

// LiteLLM's MockOpenAITransport answers at the transport layer, so the request is still built as usual and the mock never opens a socket.
pub(crate) fn respond(request: &Request) -> Result<Response, MockError> {
    let content = request.body().and_then(Body::as_bytes).unwrap_or_default();
    let created = || {
        let elapsed = SystemTime::now().duration_since(UNIX_EPOCH);
        i64::try_from(elapsed.map_or(0, |since| since.as_secs())).unwrap_or(i64::MAX)
    };
    let mocked = handle_mock_request(content, &created, &Uuid::new_v4)
        .map_err(|source| MockError::Request { source })?;
    let body = serde_json::to_vec(&mocked.body).map_err(MockError::Encode)?;
    let answer = http::Response::builder()
        .status(mocked.status)
        .header(CONTENT_TYPE, mocked.content_type)
        .body(Bytes::from(body))
        .map_err(MockError::Response)?;
    Ok(Response::from(answer))
}

use std::time::Duration;

use reqwest::{Client, Response};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use super::AsyncStreamingConfig;
use crate::interactions::base::transport::response::response_headers;
use crate::interactions::base::{ErrorClasses, Logging, MAX_RESPONSE_BYTES};
use crate::interactions::create::{CreateInteractionError, handle_failure};
use crate::interactions::lifecycle::status_failure;
use crate::proxy::StreamResponse;
use crate::proxy::limits::read_bounded;
use crate::proxy::stream::Lease;

// LiteLLM's _STREAMING_ERROR_BODY_READ_TIMEOUT_SECONDS: a failing stream's body gets five seconds, then reads as empty
const STREAM_ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(5);

// LiteLLM's _create_async_streaming_iterator: a 2xx body stays unread, a failing one is read for its message
pub async fn create_async_iterator<C>(
    client: &Client,
    response: Response,
    model: Option<&str>,
    logging_obj: &mut Logging,
    interactions_api_config: &C,
) -> Result<C::AsyncStream, CreateInteractionError<C::Error>>
where
    C: AsyncStreamingConfig,
{
    if !response.status().is_success() {
        return Err(async_failure(interactions_api_config, response).await);
    }
    interactions_api_config
        .create_async_stream_iterator(held(client, response), model, logging_obj)
        .map_err(CreateInteractionError::Provider)
}

// the interaction clients keep no server drain tracker, so the lease carries a token of its own to hold the body
fn held(client: &Client, response: Response) -> StreamResponse {
    StreamResponse {
        response,
        lease: Lease {
            _client: client.clone(),
            _token: TaskTracker::new().token(),
        },
    }
}

// _safe_aread_response: the read gets five seconds, then a failure or an expiry reads as empty
async fn async_failure<C>(config: &C, mut response: Response) -> CreateInteractionError<C::Error>
where
    C: ErrorClasses,
{
    let status_code = response.status().as_u16();
    let headers = response_headers(response.headers());
    let read = timeout(
        STREAM_ERROR_BODY_TIMEOUT,
        read_bounded(&mut response, MAX_RESPONSE_BYTES),
    )
    .await;
    let body = read.ok().and_then(Result::ok).unwrap_or_default();
    handle_failure(config, status_failure(status_code, headers, &body))
}

use std::time::Duration;

use reqwest::blocking::Response as BlockingResponse;
use reqwest::{Client, Response};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::{ErrorClasses, Logging, MAX_RESPONSE_BYTES, UpstreamResponse};
use crate::interactions::create::{CreateInteractionError, handle_failure};
use crate::interactions::lifecycle::status_failure;
use crate::interactions::lifecycle::synchronous::finish_blocking;
use crate::proxy::StreamResponse;
use crate::proxy::limits::read_bounded;
use crate::proxy::stream::Lease;

pub(crate) mod tests;

// LiteLLM's _STREAMING_ERROR_BODY_READ_TIMEOUT_SECONDS: a failing stream's body gets five seconds, then reads as empty
const STREAM_ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(5);

// the iterators live in betterlite-core, which depends on this crate, so the provider config builds them here
pub trait SyncStreamingConfig: ErrorClasses {
    type SyncStream;

    fn create_sync_stream_iterator(
        &self,
        response: UpstreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<Self::SyncStream, Self::Error>;
}

pub trait AsyncStreamingConfig: ErrorClasses {
    type AsyncStream;

    fn create_async_stream_iterator(
        &self,
        response: StreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<Self::AsyncStream, Self::Error>;
}

// LiteLLM's _create_sync_streaming_iterator: the config builds the iterator from the whole buffered body
pub fn create_sync_iterator<C>(
    response: BlockingResponse,
    model: Option<&str>,
    logging_obj: &mut Logging,
    interactions_api_config: &C,
) -> Result<C::SyncStream, CreateInteractionError<C::Error>>
where
    C: SyncStreamingConfig,
{
    if response.status().is_success() {
        let buffered = finish_blocking(response)
            .map_err(|failure| handle_failure(interactions_api_config, failure))?;
        interactions_api_config
            .create_sync_stream_iterator(buffered, model, logging_obj)
            .map_err(CreateInteractionError::Provider)
    } else {
        Err(sync_failure(interactions_api_config, response))
    }
}

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

// _safe_read_response: a body that cannot be read reads as empty, and the real status still names the error
fn sync_failure<C>(config: &C, response: BlockingResponse) -> CreateInteractionError<C::Error>
where
    C: ErrorClasses,
{
    let status_code = response.status().as_u16();
    let headers = response_headers(response.headers());
    let body = read_body(response, MAX_RESPONSE_BYTES).unwrap_or_default();
    handle_failure(config, status_failure(status_code, headers, &body))
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

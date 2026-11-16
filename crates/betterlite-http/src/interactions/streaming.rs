use std::error::Error as StdError;

use reqwest::blocking::Response as BlockingResponse;
use reqwest::{Client, Response};
use thiserror::Error;
use tokio_util::task::TaskTracker;

use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::{
    ErrorClasses, Logging, MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse,
};
use crate::interactions::lifecycle::check_status;
use crate::proxy::StreamResponse;
use crate::proxy::limits::read_bounded;
use crate::proxy::stream::Lease;

pub(crate) mod tests;

// the iterators live in betterlite-core, which depends on this crate, so the provider config builds them here
pub trait StreamingConfig: ErrorClasses {
    type SyncStream;

    type AsyncStream;

    fn create_sync_stream_iterator(
        &self,
        response: UpstreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<Self::SyncStream, Self::Error>;

    fn create_async_stream_iterator(
        &self,
        response: StreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<Self::AsyncStream, Self::Error>;
}

#[derive(Debug, Error)]
pub enum StreamError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error(transparent)]
    Read(#[from] RequestFailure),
    #[error("{0}")]
    Provider(#[source] ProviderError),
}

// LiteLLM's _create_sync_streaming_iterator: the config builds the iterator from the whole buffered body
pub fn create_sync_iterator<C>(
    response: BlockingResponse,
    model: Option<&str>,
    logging_obj: &mut Logging,
    interactions_api_config: &C,
) -> Result<C::SyncStream, StreamError<C::Error>>
where
    C: StreamingConfig,
{
    let status_code = response.status().as_u16();
    let headers = response_headers(response.headers());
    let body = read_body(response, MAX_RESPONSE_BYTES)?;
    let buffered = check_status(status_code, headers, body)?;
    interactions_api_config
        .create_sync_stream_iterator(buffered, model, logging_obj)
        .map_err(StreamError::Provider)
}

// LiteLLM's _create_async_streaming_iterator: a 2xx body stays unread, a failing one is read for its message
pub async fn create_async_iterator<C>(
    client: &Client,
    mut response: Response,
    model: Option<&str>,
    logging_obj: &mut Logging,
    interactions_api_config: &C,
) -> Result<C::AsyncStream, StreamError<C::Error>>
where
    C: StreamingConfig,
{
    let status_code = response.status().as_u16();
    if !(200..=299).contains(&status_code) {
        let headers = response_headers(response.headers());
        let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(RequestFailure::from)?;
        if let Err(failure) = check_status(status_code, headers, body) {
            return Err(StreamError::Read(failure));
        }
    }
    // the interaction clients keep no server drain tracker, so the lease carries a token of its own to hold the body
    let source = StreamResponse {
        response,
        lease: Lease {
            _client: client.clone(),
            _token: TaskTracker::new().token(),
        },
    };
    interactions_api_config
        .create_async_stream_iterator(source, model, logging_obj)
        .map_err(StreamError::Provider)
}

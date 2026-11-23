use std::future::Future;
use std::io::Read;
use std::time::Duration;

use betterlite_http::interactions::base::{
    ClientError, MAX_RESPONSE_BYTES, Params, RequestFailure,
};
use betterlite_http::proxy::UpstreamResponse;
use http::header::HeaderMap;
use url::Url;

mod asynchronous;
mod synchronous;

#[derive(Debug)]
pub struct DecisionsRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Option<Duration>,
}

pub type Answer = Result<UpstreamResponse, RequestFailure>;

// LiteLLM's HTTPHandler: a blocking POST whose deadline is the one the caller passed, or its client's own
pub trait DecisionsSyncTransport: Send + Sync {
    fn send_sync(&mut self, request: DecisionsRequest) -> Result<Answer, ClientError>;
}

// LiteLLM's AsyncHTTPHandler: the same POST as a future, from the client get_async_httpx_client built for the provider
pub trait DecisionsAsyncTransport: Send + Sync {
    fn send_async(
        &mut self,
        params: &Params,
        request: DecisionsRequest,
    ) -> Result<impl Future<Output = Answer> + Send, ClientError>;
}

// The blocking answer, read up to the shared cap so a provider answer cannot grow without bound
pub(super) fn finish_blocking<S>(sent: S, status: u16, headers: HeaderMap) -> Answer
where
    S: Read,
{
    let cap = u64::try_from(MAX_RESPONSE_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    let mut body = Vec::new();
    sent.take(cap)
        .read_to_end(&mut body)
        .map_err(RequestFailure::Body)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(RequestFailure::BodyTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    Ok(UpstreamResponse {
        status,
        headers,
        body,
    })
}

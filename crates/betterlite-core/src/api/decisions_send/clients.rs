use std::future::Future;
use std::io::Read;
use std::time::Duration;

use betterlite_http::interactions::base::clients::{HttpClients, async_client};
use betterlite_http::interactions::base::{
    ClientError, MAX_RESPONSE_BYTES, Params, RequestFailure,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::read_bounded;
use http::header::HeaderMap;
use reqwest::Client;
use reqwest::blocking::Client as BlockingClient;
use url::Url;

#[derive(Debug)]
pub struct DecisionsRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Option<Duration>,
}

type Answer = Result<UpstreamResponse, RequestFailure>;

// LiteLLM's HTTPHandler: a blocking POST whose deadline is the one the caller passed, or its client's own
pub trait DecisionsSyncClient: Clone + Send + Sync {
    fn send_blocking(&self, request: DecisionsRequest) -> Answer;
}

// LiteLLM's AsyncHTTPHandler: the same POST as a future
pub trait DecisionsAsyncClient: Clone + Send + Sync {
    fn send(&self, request: DecisionsRequest) -> impl Future<Output = Answer> + Send;
}

// _get_httpx_client and get_async_httpx_client(llm_provider=...): the pool's client for this call
pub trait DecisionsClients: Send + Sync {
    type SyncClient: DecisionsSyncClient;
    type AsyncClient: DecisionsAsyncClient;

    fn select_sync(&mut self, params: &Params) -> Result<Self::SyncClient, ClientError>;

    fn select_async(&mut self, params: &Params) -> Result<Self::AsyncClient, ClientError>;
}

// The blocking client B173's factory built for this call
#[derive(Clone)]
pub struct SyncSender(BlockingClient);

impl DecisionsSyncClient for SyncSender {
    fn send_blocking(&self, request: DecisionsRequest) -> Answer {
        let mut builder = self.0.post(request.url).headers(request.headers);
        if let Some(deadline) = request.timeout {
            builder = builder.timeout(deadline);
        }
        let sent = builder
            .body(request.body)
            .send()
            .map_err(RequestFailure::Transport)?;
        let status = sent.status().as_u16();
        let headers = sent.headers().clone();
        finish_blocking(sent, status, headers)
    }
}

#[derive(Clone)]
pub struct AsyncSender(Client);

impl DecisionsAsyncClient for AsyncSender {
    fn send(&self, request: DecisionsRequest) -> impl Future<Output = Answer> + Send {
        let client = self.0.clone();
        async move {
            let mut builder = client
                .post(request.url)
                .headers(request.headers)
                .body(request.body);
            if let Some(timeout) = request.timeout {
                builder = builder.timeout(timeout);
            }
            let mut sent = builder.send().await.map_err(RequestFailure::Transport)?;
            let status = sent.status().as_u16();
            let headers = sent.headers().clone();
            let body = read_bounded(&mut sent, MAX_RESPONSE_BYTES)
                .await
                .map_err(RequestFailure::from)?;
            Ok(UpstreamResponse {
                status,
                headers,
                body,
            })
        }
    }
}

impl DecisionsClients for HttpClients {
    type AsyncClient = AsyncSender;
    type SyncClient = SyncSender;

    fn select_sync(&mut self, params: &Params) -> Result<SyncSender, ClientError> {
        let client = self.sync_client(params, None)?;
        Ok(SyncSender(client))
    }

    fn select_async(&mut self, params: &Params) -> Result<AsyncSender, ClientError> {
        let client = async_client(self, params, None)?;
        Ok(AsyncSender(client))
    }
}

// The blocking answer, read up to the shared cap so a provider answer cannot grow without bound
pub(crate) fn finish_blocking<S>(sent: S, status: u16, headers: HeaderMap) -> Answer
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

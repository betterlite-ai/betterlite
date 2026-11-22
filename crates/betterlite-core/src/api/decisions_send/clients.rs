use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::interactions::base::clients::{HttpClients, async_client};
use betterlite_http::interactions::base::{
    ClientError, MAX_RESPONSE_BYTES, Params, RequestFailure,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::read_bounded;
use http::header::HeaderMap;
use url::Url;

// The body a decisions call carries: the signed bytes when sign_request built them, else the JSON copy
#[derive(Debug)]
pub enum DecisionsBody {
    Json(Vec<u8>),
    Signed(Vec<u8>),
}

// The wire request both client kinds POST
#[derive(Debug)]
pub struct DecisionsRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: DecisionsBody,
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

// The blocking client B173's factory built for this call, held as the send itself
#[derive(Clone)]
pub struct SyncSender(Arc<BlockingSend>);

type BlockingSend = dyn Fn(DecisionsRequest) -> Answer + Send + Sync;

impl DecisionsSyncClient for SyncSender {
    fn send_blocking(&self, request: DecisionsRequest) -> Answer {
        (self.0)(request)
    }
}

// The async client B173's factory built for this call, held the same way
#[derive(Clone)]
pub struct AsyncSender(Arc<AsyncSend>);

type AsyncAnswer = Pin<Box<dyn Future<Output = Answer> + Send>>;
type AsyncSend = dyn Fn(DecisionsRequest) -> AsyncAnswer + Send + Sync;

impl DecisionsAsyncClient for AsyncSender {
    fn send(&self, request: DecisionsRequest) -> impl Future<Output = Answer> + Send {
        (self.0)(request)
    }
}

impl DecisionsClients for HttpClients {
    type AsyncClient = AsyncSender;
    type SyncClient = SyncSender;

    fn select_sync(&mut self, params: &Params) -> Result<SyncSender, ClientError> {
        sync_sender(self, params)
    }

    fn select_async(&mut self, params: &Params) -> Result<AsyncSender, ClientError> {
        async_sender(self, params)
    }
}

// The blocking sender: one request, sent once, its reply read under the shared cap
fn sync_sender(clients: &mut HttpClients, params: &Params) -> Result<SyncSender, ClientError> {
    let client = clients.sync_client(params, None)?;
    Ok(SyncSender(Arc::new(
        move |request: DecisionsRequest| -> Answer {
            let DecisionsRequest {
                url,
                headers,
                body,
                timeout,
            } = request;
            let mut builder = client.post(url).headers(headers);
            if let Some(deadline) = timeout {
                builder = builder.timeout(deadline);
            }
            let sent = builder
                .body(body_bytes(body))
                .send()
                .map_err(RequestFailure::Transport)?;
            let status = sent.status().as_u16();
            let reply_headers = sent.headers().clone();
            finish_blocking(sent, status, reply_headers)
        },
    )))
}

// The async sender: the same POST sent as a future, read under the same cap
fn async_sender(clients: &mut HttpClients, params: &Params) -> Result<AsyncSender, ClientError> {
    let client = async_client(clients, params, None)?;
    Ok(AsyncSender(Arc::new(
        move |request: DecisionsRequest| -> AsyncAnswer {
            let cloned = client.clone();
            Box::pin(async move {
                let mut builder = cloned
                    .post(request.url)
                    .headers(request.headers)
                    .body(body_bytes(request.body));
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
            })
        },
    )))
}

// A signed body goes out as content and an unsigned one as JSON; on the wire both are the request body
fn body_bytes(body: DecisionsBody) -> Vec<u8> {
    match body {
        DecisionsBody::Json(bytes) | DecisionsBody::Signed(bytes) => bytes,
    }
}

// The blocking answer, read up to the shared cap so a provider answer cannot grow without bound
fn finish_blocking<S>(sent: S, status: u16, headers: HeaderMap) -> Answer
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

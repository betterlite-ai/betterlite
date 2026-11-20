use std::future::Future;
use std::io::Read as _;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::interactions::base::clients::{HttpClients, async_client};
use betterlite_http::interactions::base::{
    ClientError, MAX_RESPONSE_BYTES, Params, RequestFailure,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::read_bounded;
use http::Method;
use http::header::HeaderMap;
use url::Url;

// the body a character call carries: a lookup sends none, and a creation sends the encoded parts, with the request's
// Content-Type naming the boundary they were encoded with
#[derive(Debug)]
pub enum CharacterBody {
    Empty,
    Multipart(Vec<u8>),
}

impl CharacterBody {
    // the bytes to send, when the call carries a body
    fn into_data(self) -> Option<Vec<u8>> {
        match self {
            Self::Empty => None,
            Self::Multipart(data) => Some(data),
        }
    }
}

// the wire request both client kinds send
#[derive(Debug)]
pub struct CharacterRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: CharacterBody,
    pub timeout: Option<Duration>,
}

type Answer = Result<UpstreamResponse, RequestFailure>;

// LiteLLM's HTTPHandler: a blocking sender whose deadline is the one the caller passed, or its client's own
pub trait CharacterSyncClient: Clone + Send + Sync {
    fn send_blocking(&self, request: CharacterRequest) -> Answer;
}

// LiteLLM's AsyncHTTPHandler: the same send as a future
pub trait CharacterAsyncClient: Clone + Send + Sync {
    fn send(&self, request: CharacterRequest) -> impl Future<Output = Answer> + Send;
}

// the client a caller handed in, split by the class it is; a sync call ignores an async-only supplied client, as isinstance does
pub struct SuppliedClients<'call, S>
where
    S: CharacterClients,
{
    pub sync: Option<&'call S::SyncClient>,
    pub asynchronous: Option<&'call S::AsyncClient>,
}

// _get_httpx_client and get_async_httpx_client: the supplied client when it is the class this call sends with, else one built for the call
pub trait CharacterClients: Send + Sync {
    type SyncClient: CharacterSyncClient;
    type AsyncClient: CharacterAsyncClient;

    fn select_sync(
        &mut self,
        params: &Params,
        supplied: Option<&Self::SyncClient>,
    ) -> Result<Self::SyncClient, ClientError>;

    fn select_async(
        &mut self,
        params: &Params,
        supplied: Option<&Self::AsyncClient>,
    ) -> Result<Self::AsyncClient, ClientError>;
}

type BlockingSend = dyn Fn(CharacterRequest) -> Answer + Send + Sync;

// the blocking client B173's factory built for this call, held as the send itself so no client type is named here
#[derive(Clone)]
pub struct SyncSender(Arc<BlockingSend>);

impl CharacterSyncClient for SyncSender {
    fn send_blocking(&self, request: CharacterRequest) -> Answer {
        (self.0)(request)
    }
}

type AsyncAnswer = Pin<Box<dyn Future<Output = Answer> + Send>>;
type AsyncSend = dyn Fn(CharacterRequest) -> AsyncAnswer + Send + Sync;

// the async client B173's factory built for this call, held the same way
#[derive(Clone)]
pub struct AsyncSender(Arc<AsyncSend>);

impl CharacterAsyncClient for AsyncSender {
    fn send(&self, request: CharacterRequest) -> impl Future<Output = Answer> + Send {
        (self.0)(request)
    }
}

impl CharacterClients for HttpClients {
    type AsyncClient = AsyncSender;
    type SyncClient = SyncSender;

    fn select_sync(
        &mut self,
        params: &Params,
        supplied: Option<&SyncSender>,
    ) -> Result<SyncSender, ClientError> {
        if let Some(client) = supplied {
            return Ok(client.clone());
        }
        let client = self.sync_client(params, None)?;
        Ok(SyncSender(Arc::new(
            move |request: CharacterRequest| -> Answer {
                let mut builder = client
                    .request(request.method, request.url)
                    .headers(request.headers);
                if let Some(timeout) = request.timeout {
                    builder = builder.timeout(timeout);
                }
                if let Some(data) = request.body.into_data() {
                    builder = builder.body(data);
                }
                let sent = builder.send().map_err(RequestFailure::Transport)?;
                let status = sent.status().as_u16();
                let headers = sent.headers().clone();
                finish_blocking(sent, status, headers)
            },
        )))
    }

    fn select_async(
        &mut self,
        params: &Params,
        supplied: Option<&AsyncSender>,
    ) -> Result<AsyncSender, ClientError> {
        if let Some(client) = supplied {
            return Ok(client.clone());
        }
        let client = async_client(self, params, None)?;
        Ok(AsyncSender(Arc::new(
            move |request: CharacterRequest| -> AsyncAnswer {
                let client = client.clone();
                Box::pin(async move {
                    let mut builder = client
                        .request(request.method, request.url)
                        .headers(request.headers);
                    if let Some(timeout) = request.timeout {
                        builder = builder.timeout(timeout);
                    }
                    if let Some(data) = request.body.into_data() {
                        builder = builder.body(data);
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
}

// the blocking answer, read up to the shared cap so a provider answer cannot grow without bound
fn finish_blocking(sent: impl Read, status: u16, headers: HeaderMap) -> Answer {
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

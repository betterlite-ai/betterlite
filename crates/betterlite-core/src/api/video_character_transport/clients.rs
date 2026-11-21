use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{ClientError, Params, RequestFailure};
use betterlite_http::proxy::UpstreamResponse;
use http::Method;
use http::header::HeaderMap;
use url::Url;

mod asynchronous;
mod synchronous;

use self::asynchronous::async_sender;
use self::synchronous::sync_sender;

// the body a character call carries: a lookup sends none, and a creation sends the encoded parts its Content-Type names
#[derive(Debug)]
pub enum CharacterBody {
    Empty,
    Multipart(Vec<u8>),
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
        supplied.map_or_else(|| sync_sender(self, params), |client| Ok(client.clone()))
    }

    fn select_async(
        &mut self,
        params: &Params,
        supplied: Option<&AsyncSender>,
    ) -> Result<AsyncSender, ClientError> {
        supplied.map_or_else(|| async_sender(self, params), |client| Ok(client.clone()))
    }
}

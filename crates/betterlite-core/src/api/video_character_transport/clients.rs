use std::future::Future;
use std::io::Read as _;
use std::time::Duration;

use betterlite_http::interactions::base::clients::{HttpClients, async_client};
use betterlite_http::interactions::base::{
    ClientError, MAX_RESPONSE_BYTES, Params, RequestFailure,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::read_bounded;
use reqwest::blocking::Client as BlockingClient;
use reqwest::blocking::Response as BlockingResponse;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Client, Method, Url};

// the body a character call carries: a lookup sends none, and a creation sends the transform's multipart parts
#[derive(Clone, Debug)]
pub enum CharacterBody {
    Empty,
    Multipart { content_type: String, data: Vec<u8> },
}

// the wire request both client kinds send
#[derive(Clone, Debug)]
pub struct CharacterRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: CharacterBody,
    pub timeout: Option<Duration>,
}

// LiteLLM's HTTPHandler: a blocking sender whose deadline is the one the caller passed, or none at all
pub trait CharacterSyncClient: Clone + Send + Sync {
    fn send_blocking(&self, request: CharacterRequest) -> Result<UpstreamResponse, RequestFailure>;
}

// LiteLLM's AsyncHTTPHandler: the same send as a future
pub trait CharacterAsyncClient: Clone + Send + Sync {
    fn send(
        &self,
        request: CharacterRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, RequestFailure>> + Send;
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

impl CharacterClients for HttpClients {
    type AsyncClient = Client;
    type SyncClient = BlockingClient;

    fn select_sync(
        &mut self,
        params: &Params,
        supplied: Option<&BlockingClient>,
    ) -> Result<BlockingClient, ClientError> {
        HttpClients::sync_client(self, params, supplied.cloned())
    }

    fn select_async(
        &mut self,
        params: &Params,
        supplied: Option<&Client>,
    ) -> Result<Client, ClientError> {
        async_client(self, params, supplied.cloned())
    }
}

impl CharacterSyncClient for BlockingClient {
    fn send_blocking(&self, request: CharacterRequest) -> Result<UpstreamResponse, RequestFailure> {
        let mut builder = self
            .request(request.method, request.url)
            .headers(request.headers);
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        let built = match request.body {
            CharacterBody::Empty => builder,
            CharacterBody::Multipart { content_type, data } => builder
                .header(
                    CONTENT_TYPE,
                    HeaderValue::from_bytes(content_type.as_bytes())
                        .map_err(RequestFailure::HeaderValue)?,
                )
                .body(data),
        };
        let sent = built.send().map_err(RequestFailure::Transport)?;
        finish_blocking(sent)
    }
}

impl CharacterAsyncClient for Client {
    async fn send(&self, request: CharacterRequest) -> Result<UpstreamResponse, RequestFailure> {
        let mut builder = self
            .request(request.method, request.url)
            .headers(request.headers);
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        let built = match request.body {
            CharacterBody::Empty => builder,
            CharacterBody::Multipart { content_type, data } => builder
                .header(
                    CONTENT_TYPE,
                    HeaderValue::from_bytes(content_type.as_bytes())
                        .map_err(RequestFailure::HeaderValue)?,
                )
                .body(data),
        };
        let mut sent = built.send().await.map_err(RequestFailure::Transport)?;
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

// the blocking answer, read up to the shared cap so a provider answer cannot grow without bound
fn finish_blocking(sent: BlockingResponse) -> Result<UpstreamResponse, RequestFailure> {
    let status = sent.status().as_u16();
    let headers = sent.headers().clone();
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

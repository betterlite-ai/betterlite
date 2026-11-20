use std::error::Error as StdError;
use std::future::Future;
use std::str::FromStr as _;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::identity::provider_id::ProviderId;
use http::{HeaderMap, Method};
use url::Url;

use super::error::VideoRemixListTransportError;
use super::fields;
use super::records::Params;

// One provider call's wire request, before a sync or async client puts it on the network
pub struct VideoRequest {
    pub method: Method,
    pub url: Url,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub body: RequestBody,
    pub timeout: Option<Duration>,
}

// LiteLLM's sync HTTPHandler: a blocking post or get
pub trait VideoSyncClient: Send + Sync {
    fn send(&self, request: VideoRequest) -> Result<UpstreamResponse, TransportError>;
}

// LiteLLM's AsyncHTTPHandler: the same send, awaited, with the client's own timeout for a call that names none
pub trait VideoAsyncClient: Send + Sync {
    fn default_timeout(&self) -> Duration;

    fn send(
        &self,
        request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

// The B173 pool: one sync client per TLS setting, one async client per provider name and TLS setting
pub trait VideoClientSource: Send + Sync {
    type SyncClient: VideoSyncClient;
    type AsyncClient: VideoAsyncClient;
    type Error: StdError + Send + Sync + 'static;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Self::SyncClient, Self::Error>;

    fn async_client(
        &self,
        provider: ProviderId,
        ssl_verify: Option<bool>,
    ) -> Result<Self::AsyncClient, Self::Error>;
}

// The client a call carries: a handler uses it only when its mode matches the path it runs
pub enum VideoClient<'call, Source: VideoClientSource> {
    Sync(&'call Source::SyncClient),
    Asynchronous(&'call Source::AsyncClient),
}

pub(super) enum Chosen<'call, Client> {
    Supplied(&'call Client),
    Pooled(Client),
}

impl<Client> Chosen<'_, Client> {
    pub(super) const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Pooled(client) => client,
        }
    }
}

// The async path's client: a supplied one of the async kind, or the pool's client for the provider name, which fails before the try block
pub(super) fn choose_async<'call, Source: VideoClientSource>(
    source: &Source,
    supplied: Option<&'call Source::AsyncClient>,
    provider: &str,
    params: &Params,
) -> Result<Chosen<'call, Source::AsyncClient>, VideoRemixListTransportError> {
    let Some(client) = supplied else {
        let parsed = ProviderId::from_str(provider).map_err(|_unknown| {
            VideoRemixListTransportError::UnknownProvider {
                provider: provider.to_owned(),
            }
        })?;
        let ssl_verify = fields::read::<bool>(params, "ssl_verify")?;
        let pooled = source.async_client(parsed, ssl_verify).map_err(|source| {
            VideoRemixListTransportError::Client {
                source: Box::new(source),
            }
        })?;
        return Ok(Chosen::Pooled(pooled));
    };
    Ok(Chosen::Supplied(client))
}

// The pooled async client is the B173 transport: the request's timeout or the client's read timeout sets the deadline
impl VideoAsyncClient for ProxiedClient {
    fn default_timeout(&self) -> Duration {
        self.limits.read_timeout
    }

    fn send(
        &self,
        request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let deadline = request.timeout.unwrap_or(self.default_timeout());
        let limit = self.limits.body_bytes;
        async move {
            self.send_following(OutboundRequest {
                method: request.method,
                url: request.url,
                query: request.query,
                headers: request.headers,
                body: request.body,
                deadline,
            })
            .await?
            .buffered(limit)
            .await
        }
    }
}

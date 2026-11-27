use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_core::api::video_remix_list_transport::client::{
    VideoAsyncClient, VideoClientSource, VideoRequest, VideoSyncClient,
};
use betterlite_core::api::video_status_transport::clients::{
    AsyncClient as StatusAsyncClient, ClientFactories, SyncClient as StatusSyncClient,
};
use betterlite_core::api::video_status_transport::config::VideoErrorClasses;
use betterlite_core::api::video_status_transport::error::{
    BaseLlmException, ErrorConverter, StatusFailure,
};
use betterlite_core::api::video_status_transport::records::{
    Headers as StatusHeaders, StatusMethod, StatusRequest,
};
use betterlite_core::api::videos_http_transport::client::{
    ClientError, ClientRequest, ClientSource,
};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::identity::provider_id::ProviderId;
use http::Method;
use http::header::HeaderMap;
use pyo3_async_runtimes::tokio::get_runtime;
use url::Url;

// the B173 pool behind one session: the default client plus the per-ssl_verify clients it builds
pub(crate) struct NativeSource {
    environment: BTreeMap<String, String>,
    policy: Arc<ProxyPolicy>,
    default: ProxiedClient,
}

impl NativeSource {
    pub(crate) fn new(environment: &BTreeMap<String, String>, default: &ProxiedClient) -> Self {
        Self {
            environment: environment.clone(),
            policy: Arc::clone(&default.policy),
            default: default.clone(),
        }
    }

    fn client_with(&self, ssl_verify: Option<bool>) -> Result<ProxiedClient, PoolError> {
        match ssl_verify {
            Some(false) => {
                let tls = TlsPolicy {
                    verification: Verification::Disabled,
                    ca_file: None,
                    certificate: None,
                    http2: false,
                    ecdh_curve: None,
                    user_agent: user_agent(&self.environment),
                };
                ProxiedClient::outbound_with_tls(&self.policy, &tls).map_err(PoolError::Client)
            }
            Some(true) | None => Ok(self.default.clone()),
        }
    }
}

// TlsPolicy's default agent, read the way every family reads LITELLM_USER_AGENT
fn user_agent(environment: &BTreeMap<String, String>) -> String {
    environment
        .get("LITELLM_USER_AGENT")
        .cloned()
        .unwrap_or_else(|| format!("betterlite/{}", env!("CARGO_PKG_VERSION")))
}

impl ClientSource for NativeSource {
    type Client = ProxiedClient;

    fn client(&self, request: ClientRequest<'_>) -> Result<Self::Client, ClientError> {
        self.client_with(request.ssl_verify)
            .map_err(ClientError::Pool)
    }
}

// LiteLLM's blocking HTTPHandler over the pooled client: the native sync calls block on this one
pub(crate) struct BlockingClient(pub(crate) ProxiedClient);

impl VideoSyncClient for BlockingClient {
    fn send(&self, request: VideoRequest) -> Result<UpstreamResponse, TransportError> {
        get_runtime().block_on(VideoAsyncClient::send(&self.0, request))
    }
}

impl VideoClientSource for NativeSource {
    type SyncClient = BlockingClient;
    type AsyncClient = ProxiedClient;
    type Error = ClientError;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Self::SyncClient, Self::Error> {
        Ok(BlockingClient(
            self.client_with(ssl_verify).map_err(ClientError::Pool)?,
        ))
    }

    fn async_client(
        &self,
        _provider: ProviderId,
        ssl_verify: Option<bool>,
    ) -> Result<Self::AsyncClient, Self::Error> {
        self.client_with(ssl_verify).map_err(ClientError::Pool)
    }
}

// the status transport's two client handles, both over the pooled client
pub(crate) struct StatusClient(pub(crate) ProxiedClient);

impl StatusClient {
    async fn send_async(
        &self,
        request: &StatusRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        let url =
            Url::parse(&request.url).map_err(|source| TransportError::RedirectUrl { source })?;
        let method = match request.method {
            StatusMethod::Get => Method::GET,
            StatusMethod::Post => Method::POST,
        };
        let body = match &request.body {
            Some(params) => {
                let encoded =
                    serde_json::to_vec(params).map_err(|_encoding| TransportError::Closed)?;
                RequestBody::Json(encoded)
            }
            None => RequestBody::Empty,
        };
        self.0
            .send_following(OutboundRequest {
                method,
                url,
                query: Vec::new(),
                headers: header_map(&self.0, request)?,
                body,
                deadline: self.0.limits.read_timeout,
            })
            .await?
            .buffered(self.0.limits.body_bytes)
            .await
    }
}

fn header_map(
    client: &ProxiedClient,
    request: &StatusRequest,
) -> Result<HeaderMap, TransportError> {
    let mut builder = client.client.request(Method::GET, &request.url);
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let built = builder.build().map_err(TransportError::Build)?;
    Ok(built.headers().clone())
}

impl StatusSyncClient for StatusClient {
    fn send(&self, request: &StatusRequest) -> Result<UpstreamResponse, TransportError> {
        get_runtime().block_on(self.send_async(request))
    }
}

#[async_trait]
impl StatusAsyncClient for StatusClient {
    async fn send(&self, request: &StatusRequest) -> Result<UpstreamResponse, TransportError> {
        self.send_async(request).await
    }
}

pub(crate) struct StatusFactories {
    pub(crate) source: NativeSource,
}

impl ClientFactories for StatusFactories {
    type Synchronous = StatusClient;
    type Asynchronous = StatusClient;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Self::Synchronous, PoolError> {
        Ok(StatusClient(self.source.client_with(ssl_verify)?))
    }

    fn async_client(
        &self,
        _provider: ProviderId,
        ssl_verify: Option<bool>,
    ) -> Result<Self::Asynchronous, PoolError> {
        Ok(StatusClient(self.source.client_with(ssl_verify)?))
    }
}

// B2402's shared _handle_error: the failure's facts, then the provider's own error class
pub(crate) struct StatusConverter;

impl ErrorConverter for StatusConverter {
    fn convert(
        &self,
        config: &dyn VideoErrorClasses,
        failure: StatusFailure<'_>,
    ) -> BaseLlmException {
        let (message, status, headers) = match failure {
            StatusFailure::Send(error) => (error.to_string(), None, StatusHeaders::new()),
            StatusFailure::Transform(error) => (
                error.message.clone(),
                error.status_code,
                error.headers.clone().unwrap_or_default(),
            ),
        };
        let synthesized = status.is_none();
        let mut class = config.get_error_class(&message, status.unwrap_or(500_u16), &headers);
        class.status_code_is_synthesized = synthesized;
        class
    }
}

mod tests;

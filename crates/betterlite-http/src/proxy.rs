use std::collections::BTreeMap;
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::{ProxyPolicy, ProxyRoute, ProxyServer};
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ModelDeployment};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap};
use reqwest::redirect::Policy;
use reqwest::retry::never;
use reqwest::{Client, ClientBuilder, Proxy, Request, Response, Url};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::error::HttpError;
use crate::proxy::limits::{ReadError, TransportLimits, check_headers, read_bounded};
use crate::proxy::outbound::OutboundRequest;
use crate::proxy::transport_error::TransportError;
use crate::tls::{TlsError, apply_policy};

pub mod limits;
pub mod multipart;
pub mod outbound;
pub mod pool;
pub mod stream;
pub mod transport_error;

const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const POOL_MAX_IDLE_PER_HOST: usize = 32;

#[derive(Debug, Clone)]
pub struct ProxiedClient {
    pub client: Client,
    pub policy: Arc<ProxyPolicy>,
    pub limits: TransportLimits,
    pub tracker: TaskTracker,
}

pub fn base_builder() -> ClientBuilder {
    ClientBuilder::new()
        .tls_backend_rustls()
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
}

fn forward_proxy(policy: &Arc<ProxyPolicy>, server: &Arc<ProxyServer>) -> Proxy {
    let target = format!("http://{}:{}", server.host, server.port);
    let routing = Arc::clone(policy);
    let chosen = Arc::clone(server);
    let proxy = Proxy::custom(move |url| match routing.route(url) {
        Some(ProxyRoute::Forward(found)) if Arc::ptr_eq(found, &chosen) => Some(target.clone()),
        Some(
            ProxyRoute::Forward(_) | ProxyRoute::Unsupported { .. } | ProxyRoute::Invalid { .. },
        )
        | None => None,
    });
    match &server.credentials {
        Some(credentials) => {
            proxy.basic_auth(&credentials.username, credentials.password.expose_secret())
        }
        None => proxy,
    }
}

impl ProxiedClient {
    pub fn outbound_for_deployment(
        proxy: &Arc<ProxyPolicy>,
        config: &Config,
        deployment: &ModelDeployment,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, TlsError> {
        let tls = TlsPolicy::resolve(config, environment, deployment.litellm_params.ssl_verify)?;
        let builder = apply_policy(base_builder(), &tls)?;
        Self::build(builder, proxy, TransportLimits::DEFAULT).map_err(TlsError::Http)
    }

    pub fn outbound(policy: &Arc<ProxyPolicy>) -> Result<Self, HttpError> {
        Self::build(
            base_builder().http1_only(),
            policy,
            TransportLimits::DEFAULT,
        )
    }

    pub fn outbound_with_tls(policy: &Arc<ProxyPolicy>, tls: &TlsPolicy) -> Result<Self, TlsError> {
        let builder = apply_policy(base_builder(), tls)?;
        let client =
            Self::build(builder, policy, TransportLimits::DEFAULT).map_err(TlsError::Http)?;
        Ok(client)
    }

    pub fn build(
        builder: ClientBuilder,
        policy: &Arc<ProxyPolicy>,
        limits: TransportLimits,
    ) -> Result<Self, HttpError> {
        for warning in &policy.warnings {
            tracing::warn!(?warning, "proxy setting ignored");
        }
        let header_list_cap = u32::try_from(limits.header_bytes).unwrap_or(u32::MAX);
        // reqwest would otherwise read the process environment, retry protocol NACKs and follow redirects with the provider key attached
        let mut configured = builder
            .no_proxy()
            .retry(never())
            .redirect(Policy::none())
            .connect_timeout(limits.connect_timeout)
            .read_timeout(limits.read_timeout)
            .http2_max_header_list_size(header_list_cap);
        for route in policy.primary.values().chain(policy.fallback.values()) {
            if let ProxyRoute::Forward(server) = route {
                configured = configured.proxy(forward_proxy(policy, server));
            }
        }
        let client = configured.build().map_err(HttpError::Client)?;
        Ok(Self {
            client,
            policy: Arc::clone(policy),
            limits,
            tracker: TaskTracker::new(),
        })
    }

    pub(crate) fn rejected(&self, url: &Url) -> Option<HttpError> {
        match self.policy.route(url) {
            Some(ProxyRoute::Unsupported { scheme }) => Some(HttpError::UnsupportedProxyScheme {
                scheme: scheme.clone(),
            }),
            Some(ProxyRoute::Invalid { name }) => {
                Some(HttpError::InvalidProxyUrl { name: name.clone() })
            }
            Some(ProxyRoute::Forward(_)) | None => None,
        }
    }

    pub async fn execute(&self, request: Request) -> Result<Response, HttpError> {
        if let Some(rejection) = self.rejected(request.url()) {
            return Err(rejection);
        }
        self.client
            .execute(request)
            .await
            .map_err(HttpError::Request)
    }

    pub async fn drain(&self, deadline: Duration) -> Result<(), TransportError> {
        self.tracker.close();
        timeout(deadline, self.tracker.wait())
            .await
            .map_err(|_elapsed| TransportError::DrainTimeout {
                remaining: self.tracker.len(),
            })
    }

    pub async fn send(&self, request: OutboundRequest) -> Result<StreamResponse, TransportError> {
        if self.tracker.is_closed() {
            return Err(TransportError::Closed);
        }
        let token = self.tracker.token();
        let built = request.build(&self.client)?;
        if let Some(rejection) = self.rejected(built.url()) {
            return Err(TransportError::Rejected(rejection));
        }
        let response = self
            .client
            .execute(built)
            .await
            .map_err(TransportError::sending)?;
        check_headers(response.headers(), self.limits.header_bytes)?;
        Ok(StreamResponse {
            response,
            lease: stream::Lease {
                _client: self.client.clone(),
                _token: token,
            },
        })
    }

    pub async fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        self.send(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }

    pub async fn post_json(&self, post: JsonPost<'_>) -> Result<UpstreamResponse, HttpError> {
        let _token = self.tracker.token();
        let body_limit = post.body_limit;
        let request = self.json_request(post, "application/json")?;
        let mut response = self.execute(request).await.map_err(timed_out)?;
        check_headers(response.headers(), self.limits.header_bytes).map_err(|exceeded| {
            HttpError::TooLarge {
                limit: exceeded.limit,
            }
        })?;
        let status = response.status().as_u16();
        let body =
            read_bounded(&mut response, body_limit)
                .await
                .map_err(|failure| match failure {
                    ReadError::TooLarge { limit } => HttpError::TooLarge { limit },
                    ReadError::Failed(source) => timed_out(HttpError::Request(source)),
                })?;
        Ok(UpstreamResponse {
            status,
            headers: mem::take(response.headers_mut()),
            body,
        })
    }

    pub fn json_request(
        &self,
        post: JsonPost<'_>,
        accept: &'static str,
    ) -> Result<Request, HttpError> {
        let mut builder = self
            .client
            .post(post.url)
            .timeout(post.timeout)
            .bearer_auth(post.bearer.expose_secret())
            .header(ACCEPT, accept)
            .header(CONTENT_TYPE, "application/json");
        for (name, value) in post.headers {
            builder = builder.header(name, value);
        }
        builder.body(post.body).build().map_err(HttpError::Request)
    }
}

#[must_use]
pub fn timed_out(error: HttpError) -> HttpError {
    if let HttpError::Request(source) = &error
        && source.is_timeout()
    {
        HttpError::Timeout
    } else {
        error
    }
}

pub struct JsonPost<'key> {
    pub url: Url,
    pub bearer: &'key SecretString,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
    pub body_limit: usize,
}

#[derive(Debug)]
pub struct UpstreamResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub struct StreamResponse {
    pub response: Response,
    pub lease: stream::Lease,
}

mod tests;

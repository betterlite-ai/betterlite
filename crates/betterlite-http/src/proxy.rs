use std::mem;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::{ProxyPolicy, ProxyRoute};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap};
use reqwest::{Client, ClientBuilder, Proxy, Request, Response, Url};
use secrecy::{ExposeSecret as _, SecretString};

use crate::error::HttpError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const POOL_MAX_IDLE_PER_HOST: usize = 32;

#[derive(Debug)]
pub struct ProxiedClient {
    pub client: Client,
    pub policy: Arc<ProxyPolicy>,
}

impl ProxiedClient {
    pub fn outbound(policy: &Arc<ProxyPolicy>) -> Result<Self, HttpError> {
        let builder = ClientBuilder::new()
            .tls_backend_rustls()
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST);
        Self::build(builder, policy)
    }

    pub fn build(builder: ClientBuilder, policy: &Arc<ProxyPolicy>) -> Result<Self, HttpError> {
        for warning in &policy.warnings {
            tracing::warn!(?warning, "proxy setting ignored");
        }
        // reqwest would otherwise read the process environment on its own
        let mut configured = builder.no_proxy();
        for route in policy.primary.values().chain(policy.fallback.values()) {
            let ProxyRoute::Forward(server) = route else {
                continue;
            };
            let target = format!("http://{}:{}", server.host, server.port);
            let routing = Arc::clone(policy);
            let chosen = Arc::clone(server);
            let mut proxy = Proxy::custom(move |url| match routing.route(url) {
                Some(ProxyRoute::Forward(found)) if Arc::ptr_eq(found, &chosen) => {
                    Some(target.clone())
                }
                Some(
                    ProxyRoute::Forward(_)
                    | ProxyRoute::Unsupported { .. }
                    | ProxyRoute::Invalid { .. },
                )
                | None => None,
            });
            if let Some(credentials) = &server.credentials {
                let password = credentials.password.expose_secret();
                proxy = proxy.basic_auth(&credentials.username, password);
            }
            configured = configured.proxy(proxy);
        }
        let client = configured.build().map_err(HttpError::Client)?;
        Ok(Self {
            client,
            policy: Arc::clone(policy),
        })
    }

    pub async fn execute(&self, request: Request) -> Result<Response, HttpError> {
        let rejection = match self.policy.route(request.url()) {
            Some(ProxyRoute::Unsupported { scheme }) => HttpError::UnsupportedProxyScheme {
                scheme: scheme.clone(),
            },
            Some(ProxyRoute::Invalid { name }) => HttpError::InvalidProxyUrl { name: name.clone() },
            Some(ProxyRoute::Forward(_)) | None => {
                return self
                    .client
                    .execute(request)
                    .await
                    .map_err(HttpError::Request);
            }
        };
        Err(rejection)
    }

    pub async fn post_json(&self, post: JsonPost<'_>) -> Result<UpstreamResponse, HttpError> {
        let mut builder = self
            .client
            .post(post.url)
            .timeout(post.timeout)
            .bearer_auth(post.bearer.expose_secret())
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json");
        for (name, value) in post.headers {
            builder = builder.header(name, value);
        }
        let request = builder.body(post.body).build().map_err(HttpError::Request)?;
        let mut response = self.execute(request).await.map_err(timed_out)?;
        let status = response.status().as_u16();
        let headers = mem::take(response.headers_mut());
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| timed_out(HttpError::Request(source)))?
        {
            if body.len().saturating_add(chunk.len()) > post.body_limit {
                return Err(HttpError::TooLarge {
                    limit: post.body_limit,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(UpstreamResponse {
            status,
            headers,
            body,
        })
    }
}

fn timed_out(error: HttpError) -> HttpError {
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

mod tests;

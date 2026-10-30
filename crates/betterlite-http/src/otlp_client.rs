use std::io::{self, Read as _};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use betterlite_config::proxy::{ProxyPolicy, ProxyRoute, ProxyServer};
use betterlite_config::sections::billing_metrics::Metering;

use async_trait::async_trait;
use bytes::Bytes;
use opentelemetry_http::{HttpClient, HttpError, Request, Response, ResponseBodyTooLarge};
use reqwest::Proxy;
use reqwest::blocking::Client;
use rustls::ClientConfig;
use secrecy::ExposeSecret as _;
use thiserror::Error;

use crate::error;

mod tests;
pub mod tls;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum OtlpClientError {
    #[error("could not configure mutual TLS: {source}")]
    Tls {
        #[source]
        source: tls::TlsError,
    },
    #[error("could not start the thread that builds the blocking HTTP client: {source}")]
    Thread {
        #[source]
        source: io::Error,
    },
    #[error("the thread that builds the blocking HTTP client panicked")]
    ThreadPanicked,
    #[error("could not build the blocking HTTP client: {source}")]
    Build {
        #[source]
        source: reqwest::Error,
    },
}

#[derive(Debug)]
pub struct OtlpClient {
    client: Client,
    policy: Arc<ProxyPolicy>,
}

impl OtlpClient {
    pub fn build(metering: &Metering, policy: &Arc<ProxyPolicy>) -> Result<Self, OtlpClientError> {
        let tls = tls::tls_config(metering).map_err(|source| OtlpClientError::Tls { source })?;
        Self::with_tls(tls, policy)
    }

    fn with_tls(tls: ClientConfig, policy: &Arc<ProxyPolicy>) -> Result<Self, OtlpClientError> {
        for warning in &policy.warnings {
            tracing::warn!(?warning, "proxy setting ignored");
        }
        let mut builder = Client::builder()
            .no_proxy()
            .tls_backend_preconfigured(tls)
            .timeout(REQUEST_TIMEOUT);
        for route in policy.primary.values().chain(policy.fallback.values()) {
            if let ProxyRoute::Forward(server) = route {
                builder = builder.proxy(forward_proxy(server, policy));
            }
        }
        let handle = thread::Builder::new()
            .name("otlp-metrics-client".to_owned())
            .spawn(move || builder.build())
            .map_err(|source| OtlpClientError::Thread { source })?;
        let client = handle
            .join()
            .map_err(|_panic| OtlpClientError::ThreadPanicked)?
            .map_err(|source| OtlpClientError::Build { source })?;
        Ok(Self {
            client,
            policy: Arc::clone(policy),
        })
    }
}

fn forward_proxy(server: &Arc<ProxyServer>, policy: &Arc<ProxyPolicy>) -> Proxy {
    let routing = Arc::clone(policy);
    let chosen = Arc::clone(server);
    let target = format!("http://{}:{}", server.host, server.port);
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

#[async_trait]
impl HttpClient for OtlpClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let (parts, request_body) = request.into_parts();
        let outgoing = self
            .client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(request_body)
            .build()?;
        match self.policy.route(outgoing.url()) {
            Some(ProxyRoute::Unsupported { scheme }) => {
                return Err(Box::new(error::HttpError::UnsupportedProxyScheme {
                    scheme: scheme.clone(),
                }));
            }
            Some(ProxyRoute::Invalid { name }) => {
                return Err(Box::new(error::HttpError::InvalidProxyUrl {
                    name: name.clone(),
                }));
            }
            Some(ProxyRoute::Forward(_)) | None => {}
        }
        // PeriodicReader polls this blocking transport on its dedicated exporter thread.
        bounded_response(self.client.execute(outgoing)?)
    }
}

fn bounded_response(
    mut response: reqwest::blocking::Response,
) -> Result<Response<Bytes>, HttpError> {
    let limit = u64::try_from(MAX_RESPONSE_BYTES)?;
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(Box::new(ResponseBodyTooLarge::new()));
    }
    let status = response.status();
    let headers = std::mem::take(response.headers_mut());
    let mut body = Vec::new();
    response
        .take(limit.saturating_add(1))
        .read_to_end(&mut body)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(Box::new(ResponseBodyTooLarge::new()));
    }
    let mut converted = Response::builder().status(status).body(Bytes::from(body))?;
    *converted.headers_mut() = headers;
    Ok(converted)
}

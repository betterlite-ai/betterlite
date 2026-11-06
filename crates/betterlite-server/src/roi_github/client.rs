use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::Method;
use axum::http::header::{ACCEPT, AUTHORIZATION};
use axum::http::{HeaderMap, HeaderValue};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::ClientPool;
use betterlite_roi::settings::ROISettings;
use secrecy::ExposeSecret as _;
use tokio::time::sleep;
use url::Url;

use super::error::{BuildError, SourceError, http_label};
use super::transport::Transport;

pub const ACCEPT_GITHUB_JSON: &str = "application/vnd.github+json";
pub const GITHUB_TIMEOUT: Duration = Duration::from_secs(45);

pub struct GitHub<T: Transport> {
    pub transport: T,
    pub api_url: String,
    pub headers: HeaderMap,
    pub timeout: Duration,
    pub profiles: Mutex<BTreeMap<String, Option<String>>>,
}

impl<T: Transport> GitHub<T> {
    pub fn new(settings: &ROISettings, transport: T) -> Result<Self, SourceError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static(ACCEPT_GITHUB_JSON));
        let token = settings.github_token.expose_secret();
        if !token.is_empty() {
            let mut value =
                HeaderValue::from_str(&format!("Bearer {token}")).map_err(SourceError::Token)?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        Ok(Self {
            transport,
            api_url: settings.github_api_url.trim_end_matches('/').to_owned(),
            headers,
            timeout: GITHUB_TIMEOUT,
            profiles: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn url(&self, path: &str) -> Result<Url, SourceError> {
        Url::parse(&format!(
            "{}/{}",
            self.api_url,
            path.trim_start_matches('/')
        ))
        .map_err(SourceError::Url)
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        params: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<UpstreamResponse, SourceError> {
        let url = self.url(path)?;
        let mut attempt: u32 = 0;
        loop {
            let outbound = OutboundRequest {
                method: method.clone(),
                url: url.clone(),
                query: params.to_vec(),
                headers: self.headers.clone(),
                body: body
                    .as_ref()
                    .map_or(RequestBody::Empty, |json| RequestBody::Json(json.clone())),
                deadline: self.timeout,
            };
            let response = self
                .transport
                .send(outbound)
                .await
                .map_err(SourceError::Transport)?;
            let status = response.status;
            if matches!(status, 429 | 502 | 503 | 504) && method == Method::GET && attempt < 2 {
                let delay = Duration::from_millis(500).saturating_mul(attempt.saturating_add(1));
                sleep(delay).await;
                attempt = attempt.saturating_add(1);
                continue;
            }
            if status < 400 {
                return Ok(response);
            }
            return Err(SourceError::Http {
                label: http_label(status),
                status,
            });
        }
    }

    // B173's pool owns and drains the underlying clients, so this GitHub handle has nothing to close
    pub fn close(self) {}
}

impl GitHub<ProxiedClient> {
    pub fn pooled(
        settings: &ROISettings,
        pool: &ClientPool,
        tls: &TlsPolicy,
        routing: &Arc<ProxyPolicy>,
    ) -> Result<Self, BuildError> {
        match pool.client(tls, routing) {
            Ok(client) => Self::new(settings, client).map_err(BuildError::Source),
            Err(error) => Err(BuildError::Pool(error)),
        }
    }
}

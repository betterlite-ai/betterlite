use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::gcs_response_cache::GcsResponseCacheSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::gcs_client::gcs_storage_auth::GcsBucketBase;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_logger::transport::PooledGcsClient;
use betterlite_logging::gcs_mock::auth::{GcsMockAuth, VertexAuth};
use betterlite_logging::gcs_mock::client::{GcsHttpClient as _, GcsMockClient};
use betterlite_logging::gcs_mock::request::{GcsReadRequest, GcsRequest, GcsWriteRequest};
use betterlite_logging::gcs_mock::settings::{DEFAULT_LATENCY, GcsMockSettings};
use http::header::HeaderMap;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::task::JoinSet;

use crate::s3_cache::Disconnect;
use crate::s3_cache::entry::CachedEntry;

pub mod body;
pub mod error;
mod headers;
mod tests;

use self::body::decode_body;
use self::headers::{HeaderError, build_headers};

pub const DEFAULT_BASE_URL: &str = "https://storage.googleapis.com";

// urllib.parse.quote(object_name, safe='') keeps only letters, digits and the four unreserved marks
const QUOTE_SAFE_NONE: AsciiSet = NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

#[derive(Clone)]
pub struct GcsCache<Auth>
where
    Auth: VertexAuth + Clone + Send + Sync + 'static,
{
    base: Arc<GcsBucketBase<Auth, PooledGcsClient>>,
    bucket_name: String,
    service_account_path: Option<SecretString>,
    key_prefix: String,
    base_url: String,
}

impl<Auth> GcsCache<Auth>
where
    Auth: VertexAuth + Clone + Send + Sync + 'static,
{
    pub fn new(
        settings: &GcsResponseCacheSettings,
        auth: Auth,
        client: ProxiedClient,
        base_url: Option<&str>,
        environment: Arc<dyn EnvironmentSource>,
    ) -> Self {
        // the cache never reads GCS_MOCK, so the client is always the real transport
        let gcs_client = GcsMockClient {
            settings: GcsMockSettings {
                enabled: false,
                latency: DEFAULT_LATENCY,
            },
            real: Some(PooledGcsClient { client }),
            mocks_initialized: false,
        };
        let base = GcsBucketBase {
            is_mock_mode: gcs_client.settings.enabled,
            bucket_name: Some(settings.bucket_name.clone()),
            path_service_account: settings.service_account_path.clone(),
            auth: Arc::new(GcsMockAuth {
                real: auth,
                installed: false,
            }),
            client: gcs_client,
            vertex_instances: Mutex::new(BTreeMap::new()),
            environment,
        };
        Self {
            base: Arc::new(base),
            bucket_name: settings.bucket_name.clone(),
            service_account_path: settings.service_account_path.clone(),
            key_prefix: key_prefix(&settings.path),
            base_url: base_url
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_owned(),
        }
    }

    async fn request_headers(&self) -> Result<HeaderMap, HeaderError<Auth>> {
        build_headers(
            &self.base,
            self.bucket_name.clone(),
            self.service_account_path.clone(),
        )
        .await
    }

    pub fn set_cache(&self, runtime: &Handle, key: &str, value: &CachedEntry) {
        if Handle::try_current().is_ok() {
            tracing::warn!(
                error = "sync cache called inside async runtime",
                "GCS cache write failed"
            );
            return;
        }
        runtime.block_on(self.async_set_cache(key, value));
    }

    pub async fn async_set_cache(&self, key: &str, value: &CachedEntry) {
        let serialized = match serde_json::to_vec(value) {
            Ok(serialized) => serialized,
            Err(source) => {
                tracing::debug!(key = %key, error = %source, "GCS cache write failed");
                return;
            }
        };
        let headers = match self.request_headers().await {
            Ok(headers) => headers,
            Err(source) => {
                tracing::debug!(key = %key, error = %source, "GCS cache write failed");
                return;
            }
        };
        let name =
            utf8_percent_encode(&format!("{}{key}", self.key_prefix), &QUOTE_SAFE_NONE).to_string();
        let url = format!(
            "{}/upload/storage/v1/b/{}/o?uploadType=media&name={name}",
            self.base_url, self.bucket_name
        );
        let request = GcsRequest::Post(GcsWriteRequest {
            url,
            data: None,
            json: None,
            params: BTreeMap::new(),
            headers,
            timeout: None,
            stream: false,
            content: Some(serialized),
        });
        if let Err(source) = self.base.client.send(request).await {
            tracing::debug!(key = %key, error = %source, "GCS cache write failed");
        }
    }

    pub fn get_cache(&self, runtime: &Handle, key: &str) -> Option<CachedEntry> {
        if Handle::try_current().is_ok() {
            tracing::error!(
                error = "sync cache called inside async runtime",
                "GCS cache read failed"
            );
            return None;
        }
        let value = runtime.block_on(self.async_get_cache(key))?;
        tracing::debug!(key = %key, "GCS cache hit");
        Some(value)
    }

    pub async fn async_get_cache(&self, key: &str) -> Option<CachedEntry> {
        let headers = match self.request_headers().await {
            Ok(headers) => headers,
            Err(source) => return failed_read(key, &source),
        };
        let name =
            utf8_percent_encode(&format!("{}{key}", self.key_prefix), &QUOTE_SAFE_NONE).to_string();
        let url = format!(
            "{}/storage/v1/b/{}/o/{name}?alt=media",
            self.base_url, self.bucket_name
        );
        let request = GcsRequest::Get(GcsReadRequest {
            url,
            params: BTreeMap::new(),
            headers,
            follow_redirects: None,
        });
        let response = match self.base.client.send(request).await {
            Ok(response) => response,
            Err(source) => return failed_read(key, &source),
        };
        if response.status != 200 {
            return None;
        }
        match decode_body(&response.body) {
            Ok(value) => Some(value),
            Err(source) => failed_read(key, &source),
        }
    }

    pub async fn async_set_cache_pipeline(&self, entries: Vec<(String, CachedEntry)>) {
        let mut tasks = JoinSet::new();
        for (key, value) in entries {
            let cache = self.clone();
            tasks.spawn(async move {
                cache.async_set_cache(&key, &value).await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Err(source) = result {
                tracing::warn!(error = %source, "GCS cache pipeline failed");
            }
        }
    }
}

impl<Auth> fmt::Debug for GcsCache<Auth>
where
    Auth: VertexAuth + Clone + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcsCache")
            .field("bucket_name", &self.bucket_name)
            .field("key_prefix", &self.key_prefix)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl<Auth> Disconnect for GcsCache<Auth>
where
    Auth: VertexAuth + Clone + Send + Sync + 'static,
{
    async fn disconnect(&self) {}
}

fn key_prefix(gcs_path: &str) -> String {
    if gcs_path.is_empty() {
        return String::new();
    }
    format!("{}/", gcs_path.trim_end_matches('/'))
}

fn failed_read<E>(key: &str, source: &E) -> Option<CachedEntry>
where
    E: fmt::Display,
{
    tracing::error!(key = %key, error = %source, "GCS cache read failed");
    None
}

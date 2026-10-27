#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "S3 cache values use JSON passthrough and blocking SDK bridges"
)]

use std::time::{Duration, SystemTime};

use aws_config::meta::region::RegionProviderChain;
use aws_config::{BehaviorVersion, defaults};
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{Builder as S3ConfigBuilder, Credentials};
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_http_client::tls::{Provider, rustls_provider::CryptoMode};
use aws_smithy_types::date_time::{DateTime, Format};
use aws_types::region::Region;
use betterlite_config::sections::s3_response_cache::Settings;
use secrecy::ExposeSecret as _;
use serde_json::Value;
use std::str;
use tokio::runtime::Builder;
use tokio::task::{spawn_blocking, yield_now};

const DEFAULT_TTL: u64 = 31_536_000;

#[derive(Clone)]
pub struct S3Cache {
    client: Client,
    bucket_name: String,
    key_prefix: String,
}

impl S3Cache {
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let runtime_result = Builder::new_current_thread().enable_all().build();
        runtime_result.map_or_else(
            |_| Self::from_client(settings, Client::from_conf(S3ConfigBuilder::new().build())),
            |built_runtime| built_runtime.block_on(Self::new_async(settings)),
        )
    }

    pub async fn new_async(settings: &Settings) -> Self {
        let http_client = aws_smithy_http_client::Builder::new()
            .tls_provider(Provider::Rustls(CryptoMode::Ring))
            .build_https();
        let mut loader = defaults(BehaviorVersion::latest()).http_client(http_client);
        if let Some(region) = &settings.region_name {
            loader = loader.region(RegionProviderChain::first_try(Region::new(region.clone())));
        }
        if let (Some(access), Some(secret)) =
            (&settings.aws_access_key_id, &settings.aws_secret_access_key)
        {
            let credentials = Credentials::new(
                access.expose_secret(),
                secret.expose_secret(),
                settings
                    .aws_session_token
                    .as_ref()
                    .map(|token| token.expose_secret().to_owned()),
                None,
                "betterlite-config",
            );
            loader = loader.credentials_provider(credentials);
        }
        let shared_config = loader.load().await;
        let mut builder = S3ConfigBuilder::from(&shared_config);
        if let Some(endpoint) = &settings.endpoint_url {
            builder = builder.endpoint_url(endpoint);
        }
        Self::from_client(settings, Client::from_conf(builder.build()))
    }

    pub fn from_client(settings: &Settings, client: Client) -> Self {
        let key_prefix = settings
            .path
            .as_deref()
            .filter(|path| !path.is_empty())
            .map_or_else(String::new, |path| {
                format!("{}/", path.trim_end_matches('/'))
            });
        Self {
            client,
            bucket_name: settings.bucket_name.clone(),
            key_prefix,
        }
    }

    pub fn set_cache(&self, key: &str, value: &Value, ttl: Option<u64>) {
        let Ok(serialized) = serde_json::to_vec(value) else {
            return;
        };
        let (client, bucket, object_key) = self.request_target(key);
        let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
            return;
        };
        runtime.block_on(put_request(&client, &bucket, &object_key, serialized, ttl));
    }

    pub async fn async_set_cache(&self, key: &str, value: Value, ttl: Option<u64>) {
        let owned_key = key.to_owned();
        let cache = self.clone();
        drop(spawn_blocking(move || cache.set_cache(&owned_key, &value, ttl)).await);
    }

    #[must_use]
    pub fn get_cache(&self, key: &str) -> Option<Value> {
        if self.bucket_name.is_empty() {
            return None;
        }
        self.read_object(key)
    }

    fn read_object(&self, key: &str) -> Option<Value> {
        let (client, bucket, object_key) = self.request_target(key);
        let runtime = Builder::new_current_thread().enable_all().build().ok()?;
        runtime.block_on(async move {
            let output = client
                .get_object()
                .bucket(bucket)
                .key(object_key)
                .send()
                .await
                .ok()?;
            if output.expires_string().is_some_and(|expires_text| {
                DateTime::from_str(expires_text, Format::HttpDate)
                    .ok()
                    .and_then(|date_time| SystemTime::try_from(date_time).ok())
                    .is_some_and(|expiration| SystemTime::now() > expiration)
            }) {
                return None;
            }
            let bytes = output.body.collect().await.ok()?.into_bytes();
            decode_body(&bytes)
        })
    }

    pub async fn async_get_cache(&self, key: &str) -> Option<Value> {
        let owned_key = key.to_owned();
        let cache = self.clone();
        spawn_blocking(move || cache.get_cache(&owned_key))
            .await
            .ok()
            .flatten()
    }

    pub async fn async_set_cache_pipeline(&self, entries: Vec<(String, Value)>, ttl: Option<u64>) {
        let mut tasks = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let cache = self.clone();
            tasks.push(spawn_blocking(move || cache.set_cache(&key, &value, ttl)));
        }
        for task in tasks {
            drop(task.await);
        }
    }

    pub async fn disconnect(&self) {
        if self.bucket_name.is_empty() {
            yield_now().await;
            return;
        }
        yield_now().await;
    }

    fn object_key(&self, key: &str) -> String {
        let normalized_key = key.replace(':', "/");
        format!("{}{}", self.key_prefix, normalized_key)
    }

    fn request_target(&self, key: &str) -> (Client, String, String) {
        let client = self.client.clone();
        let bucket = self.bucket_name.clone();
        let object_key = self.object_key(key);
        (client, bucket, object_key)
    }
}

async fn put_request(client: &Client, bucket: &str, key: &str, body: Vec<u8>, ttl: Option<u64>) {
    let cache_ttl = ttl.unwrap_or(DEFAULT_TTL);
    let cache_control = format!("immutable, max-age={cache_ttl}, s-maxage={cache_ttl}");
    let mut request = client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(ByteStream::from(body))
        .cache_control(cache_control)
        .content_type("application/json")
        .content_language("en")
        .content_disposition(format!("inline; filename=\"{key}.json\""));
    if let Some(expiration_ttl) = ttl {
        let seconds = i64::try_from(expiration_ttl).unwrap_or(i64::MAX);
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        let epoch_seconds = now.saturating_add(u64::try_from(seconds).unwrap_or(0));
        let expires = DateTime::from_secs(i64::try_from(epoch_seconds).unwrap_or(i64::MAX));
        request = request.expires(expires);
    }
    if request.send().await.is_err() {
        tracing::warn!("S3 cache write failed");
    }
}

fn decode_body(bytes: &[u8]) -> Option<Value> {
    let text = str::from_utf8(bytes).ok()?;
    let parsed = serde_json::from_str(text)
        .or_else(|_| {
            let normalized = text.replace('\'', "\"");
            serde_json::from_str(&normalized)
        })
        .ok()?;
    match parsed {
        Value::Object(object) => Some(Value::Object(object)),
        Value::Array(values) => {
            let mut object = serde_json::Map::new();
            for pair in values {
                let Value::Array(mut pair_values) = pair else {
                    return None;
                };
                if pair_values.len() != 2 {
                    return None;
                }
                let value = pair_values.pop().ok_or(()).ok()?;
                let Value::String(object_key) = pair_values.pop().ok_or(()).ok()? else {
                    return None;
                };
                object.insert(object_key, value);
            }
            Some(Value::Object(object))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

#[must_use]
pub fn decode_for_fuzzing(bytes: &[u8]) -> Option<Value> {
    let mut decoded = decode_body(bytes);
    decoded.take()
}

mod tests;

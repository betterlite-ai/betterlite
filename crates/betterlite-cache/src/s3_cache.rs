use std::num::{NonZeroUsize, TryFromIntError};
use std::time::{Duration, SystemTime, SystemTimeError};

use aws_sdk_s3::Client;
use aws_sdk_s3::config::SharedHttpClient;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::DateTime;
use betterlite_config::sections::s3_response_cache::S3Settings;
use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::timeout;

pub mod body;
mod client;
#[expect(
    clippy::disallowed_types,
    reason = "the entry module contains only an opaque JSON payload newtype and its derives"
)]
pub mod entry;
mod read;
use entry::CachedEntry;

const DEFAULT_TTL: u64 = 31_536_000;

#[derive(Clone)]
pub struct S3Cache {
    client: Client,
    bucket_name: String,
    key_prefix: String,
    clock: fn() -> SystemTime,
    max_object_bytes: NonZeroUsize,
    read_timeout: Duration,
}

impl S3Cache {
    pub fn new(
        settings: &S3Settings,
        transport: SharedHttpClient,
        clock: fn() -> SystemTime,
        max_object_bytes: NonZeroUsize,
    ) -> Self {
        let client = client::build(settings, transport);
        Self {
            client,
            bucket_name: settings.bucket_name.clone(),
            key_prefix: settings.key_prefix.clone(),
            clock,
            max_object_bytes,
            read_timeout: Duration::from_secs(settings.config.read_timeout),
        }
    }

    pub fn set_cache(&self, runtime: &Handle, key: &str, value: &CachedEntry, ttl: Option<u64>) {
        if Handle::try_current().is_ok() {
            tracing::warn!(
                error = "sync cache called inside async runtime",
                "S3 cache write failed"
            );
            return;
        }
        runtime.block_on(self.async_set_cache(key, value, ttl));
    }

    pub fn get_cache(&self, runtime: &Handle, key: &str) -> Option<CachedEntry> {
        if Handle::try_current().is_ok() {
            tracing::error!(
                error = "sync cache called inside async runtime",
                "S3 cache read failed"
            );
            return None;
        }
        runtime.block_on(self.async_get_cache(key))
    }

    pub async fn async_set_cache(&self, key: &str, value: &CachedEntry, ttl: Option<u64>) {
        let serialized = match serde_json::to_vec(value) {
            Ok(body) => body,
            Err(source) => {
                tracing::warn!(error = %source, "S3 cache serialization failed");
                return;
            }
        };
        if serialized.len() > self.max_object_bytes.get() {
            tracing::warn!(error = "object exceeds size limit", "S3 cache write failed");
            return;
        }
        let expires = match self.expiration(ttl) {
            Ok(expiration) => expiration,
            Err(source) => {
                tracing::warn!(error = %source, "S3 cache write failed");
                return;
            }
        };
        self.put(key, serialized, ttl, expires).await;
    }

    async fn put(
        &self,
        key: &str,
        serialized: Vec<u8>,
        ttl: Option<u64>,
        expires: Option<DateTime>,
    ) {
        let cache_ttl = ttl.unwrap_or(DEFAULT_TTL);
        let object_key = self.object_key(key);
        let result = self
            .client
            .put_object()
            .bucket(&self.bucket_name)
            .key(&object_key)
            .body(ByteStream::from(serialized))
            .cache_control(format!(
                "immutable, max-age={cache_ttl}, s-maxage={cache_ttl}"
            ))
            .content_type("application/json")
            .content_language("en")
            .content_disposition(format!("inline; filename=\"{object_key}.json\""))
            .set_expires(expires)
            .send()
            .await;
        if let Err(source) = result {
            tracing::warn!(error = %source, "S3 cache write failed");
        }
    }

    fn expiration(&self, ttl: Option<u64>) -> Result<Option<DateTime>, CacheError> {
        let Some(seconds) = ttl else {
            return Ok(None);
        };
        let expires = (self.clock)()
            .checked_add(Duration::from_secs(seconds))
            .ok_or(CacheError::ExpirationOverflow)?;
        let timestamp = expires
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|source| CacheError::Clock { source })?;
        let epoch = i64::try_from(timestamp.as_secs())
            .map_err(|source| CacheError::Timestamp { source })?;
        // Python datetime cannot represent years beyond 9999.
        if epoch > 253_402_300_799 {
            return Err(CacheError::ExpirationOverflow);
        }
        Ok(Some(DateTime::from_secs(epoch)))
    }

    pub async fn async_set_cache_pipeline(
        &self,
        entries: Vec<(String, CachedEntry)>,
        ttl: Option<u64>,
    ) {
        let mut tasks = JoinSet::new();
        for (key, value) in entries {
            if tasks.len() >= 16
                && let Some(Err(source)) = tasks.join_next().await
            {
                tracing::warn!(error = %source, "S3 cache pipeline failed");
            }
            let cache = self.clone();
            tasks.spawn(async move {
                cache.async_set_cache(&key, &value, ttl).await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Err(source) = result {
                tracing::warn!(error = %source, "S3 cache pipeline failed");
            }
        }
    }

    pub async fn async_get_cache(&self, key: &str) -> Option<CachedEntry> {
        let output = match self
            .client
            .get_object()
            .bucket(&self.bucket_name)
            .key(self.object_key(key))
            .send()
            .await
        {
            Ok(output) => output,
            Err(source) => {
                if source
                    .as_service_error()
                    .is_some_and(GetObjectError::is_no_such_key)
                {
                    tracing::debug!(error = %source, "S3 cache miss");
                } else {
                    tracing::error!(error = %source, "S3 cache read failed");
                }
                return None;
            }
        };
        match timeout(self.read_timeout, read::read_body(self, output)).await {
            Ok(result) => result,
            Err(source) => {
                tracing::error!(error = %source, "S3 cache body timed out");
                None
            }
        }
    }

    fn object_key(&self, key: &str) -> String {
        let normalized = key.replace(':', "/");
        format!("{}{normalized}", self.key_prefix)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("S3 cache expiration overflows")]
    ExpirationOverflow,
    #[error("S3 cache clock is before epoch")]
    Clock {
        #[source]
        source: SystemTimeError,
    },
    #[error("S3 cache timestamp overflows")]
    Timestamp {
        #[source]
        source: TryFromIntError,
    },
}

pub trait Disconnect {
    fn disconnect(&self) -> impl Future<Output = ()> + Send;
}

impl Disconnect for S3Cache {
    async fn disconnect(&self) {}
}

mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::scope::active_request_redis_batch;

const NUMBER_KEY: &str = "$serde_json::private::Number";
#[derive(Debug, thiserror::Error)]
pub enum RedisRowsError {
    #[error("the request Redis read failed: {message}")]
    Batch { message: String },
    #[error("the direct Redis read failed: {message}")]
    Direct { message: String },
    #[error("the Redis pipeline write failed: {message}")]
    Write { message: String },
    #[error("no request Redis batch is open")]
    NoBatch,
}
pub type RowWrite = (String, Json, Option<f64>);
// one MGET result as the mapping the source validates; absent and JSON null both read as misses
#[async_trait]
pub trait PrefetchRedis: Send + Sync {
    async fn read(&self, keys: &[String]) -> Result<Json, RedisRowsError>;
    fn has_batch(&self) -> bool;
    async fn set_pipeline(&self, entries: &[RowWrite]) -> Result<(), RedisRowsError>;
    fn queue_set(&self, key: &str, value: &Json, ttl: Option<f64>) -> Result<(), RedisRowsError>;
}
// the source prefers the request pipeline and falls back to one direct MGET of B126's cache
pub struct RedisRows {
    pub client: Arc<RedisCache>,
}
#[async_trait]
impl PrefetchRedis for RedisRows {
    async fn read(&self, keys: &[String]) -> Result<Json, RedisRowsError> {
        let Some(batch) = active_request_redis_batch(&self.client) else {
            let wanted: Vec<Option<&str>> = keys.iter().map(|key| Some(key.as_str())).collect();
            let found = self
                .client
                .async_batch_get_cache(&wanted)
                .await
                .map_err(|error| RedisRowsError::Direct {
                    message: error.to_string(),
                })?;
            return Ok(rows_json(&found));
        };
        let found = batch
            .mget(keys)
            .await
            .map_err(|error| RedisRowsError::Batch {
                message: error.to_string(),
            })?;
        Ok(rows_json(found.as_ref()))
    }
    fn has_batch(&self) -> bool {
        active_request_redis_batch(&self.client).is_some()
    }
    async fn set_pipeline(&self, entries: &[RowWrite]) -> Result<(), RedisRowsError> {
        self.client
            .values
            .async_set_cache_pipeline_with_ttls(entries)
            .await
            .map_err(|error| RedisRowsError::Write {
                message: error.to_string(),
            })
    }
    fn queue_set(&self, key: &str, value: &Json, ttl: Option<f64>) -> Result<(), RedisRowsError> {
        let Some(batch) = active_request_redis_batch(&self.client) else {
            return Err(RedisRowsError::NoBatch);
        };
        // the request's next round trip carries the SET; the scope settles whatever is left
        drop(batch.set(key, value, ttl));
        Ok(())
    }
}
fn rows_json<'a, Values>(values: Values) -> Json
where
    Values: IntoIterator<Item = (&'a String, &'a Option<CachedValue>)>,
{
    let fields = values
        .into_iter()
        .map(|(key, value)| (key.clone(), value_json(value.as_ref())))
        .collect();
    Json::Object(fields)
}
fn value_json(value: Option<&CachedValue>) -> Json {
    match value {
        None | Some(CachedValue::Null) => Json::Null,
        Some(CachedValue::Boolean(flag)) => Json::Bool(*flag),
        Some(CachedValue::Number(number)) => Json::Number(number.clone()),
        Some(CachedValue::Text(text)) => Json::Text(text.clone()),
        Some(CachedValue::Array(items)) => {
            Json::List(items.iter().map(|item| value_json(Some(item))).collect())
        }
        Some(CachedValue::Object(entries)) => {
            // arbitrary_precision hands a bare JSON float to the untagged decoder as this one-key map
            let number: Option<serde_json::Number> =
                match (entries.len(), entries.first_key_value()) {
                    (1, Some((key, CachedValue::Text(digits)))) if key == NUMBER_KEY => {
                        digits.parse().ok()
                    }
                    _ => None,
                };
            number.map_or_else(
                || {
                    Json::Object(
                        entries
                            .iter()
                            .map(|(key, entry)| (key.clone(), value_json(Some(entry))))
                            .collect(),
                    )
                },
                Json::Number,
            )
        }
    }
}
mod tests;

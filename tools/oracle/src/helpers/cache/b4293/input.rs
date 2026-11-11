use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fields {
    pub(crate) clock: f64,
    #[serde(default)]
    pub(crate) key: String,
    #[serde(default = "default_value")]
    pub(crate) value: Json,
    #[serde(default = "default_value")]
    pub(crate) redis_get: Json,
    #[serde(default)]
    pub(crate) memory: Vec<Entry>,
    #[serde(default)]
    pub(crate) default_in_memory_ttl: Option<f64>,
    #[serde(default)]
    pub(crate) entries: Vec<Entry>,
    #[serde(default)]
    pub(crate) key_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    GetHitsMemoryFirst(Fields),
    GetBackfillsMemoryFromRedis(Fields),
    #[serde(rename = "async_get_checks_request_batch_missing")]
    BatchMissingRead(Fields),
    SetCacheAppliesDefaultTtl(Fields),
    IncrementPropagatesRedisError(Fields),
    #[serde(rename = "async_increment_fallback_on_redis_error")]
    IncrementFallback(Fields),
    SetPreCallReturnsBatchOrNone(Fields),
    SetPipelineAppliesSingleTtl(Fields),
    DeleteCacheKeysChunksRedis(Fields),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub(crate) key: String,
    pub(crate) value: Json,
}

const fn default_value() -> Json {
    Json::Null
}

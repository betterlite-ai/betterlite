use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fields {
    pub(crate) clock: f64,
    #[serde(default)]
    pub(crate) default_in_memory_ttl: Option<f64>,
    #[serde(default)]
    pub(crate) memory: Vec<Entry>,
    #[serde(default)]
    pub(crate) keys: Vec<String>,
    #[serde(default)]
    pub(crate) read_keys: Vec<String>,
    #[serde(default)]
    pub(crate) ttl_keys: Vec<String>,
    #[serde(default)]
    pub(crate) due_keys: Vec<String>,
    #[serde(default)]
    pub(crate) write_keys: Vec<String>,
    #[serde(default)]
    pub(crate) duplicate_keys: Vec<String>,
    #[serde(default)]
    pub(crate) merge_keys: Vec<String>,
    #[serde(default)]
    pub(crate) replies: Vec<Vec<Option<Json>>>,
    #[serde(default)]
    pub(crate) own_replies: Vec<Vec<Option<Json>>>,
    #[serde(default)]
    pub(crate) caches: Vec<CacheFields>,
    #[serde(default)]
    pub(crate) breaker_caches: Vec<CacheFields>,
    #[serde(default)]
    pub(crate) prepare_caches: Vec<CacheFields>,
    #[serde(default)]
    pub(crate) apply_caches: Vec<CacheFields>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub(crate) key: String,
    pub(crate) value: Json,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheFields {
    #[serde(default)]
    pub(crate) memory: Vec<Entry>,
    #[serde(default)]
    pub(crate) keys: Vec<String>,
    #[serde(default)]
    pub(crate) client: Client,
    #[serde(default)]
    pub(crate) fail_reads: bool,
    #[serde(default)]
    pub(crate) fail_writes: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Client {
    #[default]
    Shared,
    Own,
    Rejecting,
    None,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    ReserveThrottleAndRollback(Fields),
    SyncReadBackfill(Fields),
    SyncReadErrors(Fields),
    AsyncReadThrottleSwitch(Fields),
    SharedReadMerge(Fields),
    SharedReadFailures(Fields),
}

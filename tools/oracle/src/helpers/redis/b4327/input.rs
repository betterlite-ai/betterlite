use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    WriteCommands(WriteCall),
    ReadValues(ReadCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteCall {
    pub(crate) operation: WriteOperation,
    pub(crate) namespace: Option<String>,
    pub(crate) default_ttl: Option<i64>,
    pub(crate) flush_size: Option<usize>,
    #[serde(default)]
    pub(crate) entries: Vec<EntryInput>,
    #[serde(default)]
    pub(crate) nx: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WriteOperation {
    AsyncSetCache,
    AsyncSetCachePipeline,
    AsyncSetCachePipelineWithTtls,
    AsyncSetCacheSadd,
    BatchCacheWrite,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryInput {
    pub(crate) key: Option<String>,
    #[serde(default)]
    pub(crate) value: Value,
    pub(crate) ttl: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadCall {
    pub(crate) operation: ReadOperation,
    pub(crate) namespace: Option<String>,
    #[serde(default)]
    pub(crate) keys: Vec<Option<String>>,
    #[serde(default)]
    pub(crate) replies: Vec<Option<String>>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReadOperation {
    GetCache,
    BatchGetCache,
}

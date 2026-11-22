use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) chunk: Chunk,
    pub(super) enabled: bool,
    pub(super) model_name: String,
    pub(super) prices: Prices,
    pub(super) log: Option<LogFields>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Chunk {
    pub(super) kind: ChunkKind,
    pub(super) value: Option<IndexMap<String, Json>>,
    pub(super) text: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ChunkKind {
    Object,
    Text,
    Bytes,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Prices {
    pub(super) recorder: Option<Json>,
    pub(super) completion: Option<Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogFields {
    pub(super) model: String,
    pub(super) call_type: String,
    pub(super) details: IndexMap<String, Json>,
}

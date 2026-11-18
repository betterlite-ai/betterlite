use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "scenario", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    TransportArguments { runs: Vec<Run> },
    WrapperIdentity { runs: Vec<Run> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Run {
    pub(super) wrapper: Wrapper,
    pub(super) client: Client,
    pub(super) model: String,
    pub(super) api_base: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) data: IndexMap<String, Json>,
    pub(super) messages: Json,
    #[serde(default)]
    pub(super) json_mode: Option<bool>,
    #[serde(default)]
    pub(super) signed_json_body: Option<String>,
    #[serde(default)]
    pub(super) stream_chunk_size: Option<u64>,
    #[serde(default)]
    pub(super) response_headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Wrapper {
    Async,
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Client {
    None,
    Async,
    Sync,
}

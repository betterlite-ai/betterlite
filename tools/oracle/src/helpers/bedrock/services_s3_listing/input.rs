use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ListRequest,
    NextRequest,
    ListResponse,
    DeleteRequest,
    DeleteResponse,
    ContentRequest,
    Retrieve,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum WireValue {
    Text(String),
    Boolean(bool),
    Null(()),
    Other(IgnoredAny),
}

// one call shape for every operation: a case carries only the fields its operation reads
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) purpose: Option<String>,
    pub(crate) file_id: Option<String>,
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) litellm_params: Map<String, Value>,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) trusted: Option<Vec<(String, WireValue)>>,
    pub(crate) additional_args: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) requests: Vec<Call>,
    #[serde(default)]
    pub(crate) next: Vec<Call>,
    #[serde(default)]
    pub(crate) listings: Vec<Call>,
    #[serde(default)]
    pub(crate) deletes: Vec<Call>,
    #[serde(default)]
    pub(crate) contents: Vec<Call>,
    #[serde(default)]
    pub(crate) delete_responses: Vec<Call>,
    #[serde(default)]
    pub(crate) retrieves: Vec<Call>,
}

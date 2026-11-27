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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) requests: Vec<ListCall>,
    #[serde(default)]
    pub(crate) next: Vec<NextCall>,
    #[serde(default)]
    pub(crate) listings: Vec<ListingCall>,
    #[serde(default)]
    pub(crate) deletes: Vec<FileCall>,
    #[serde(default)]
    pub(crate) contents: Vec<FileCall>,
    #[serde(default)]
    pub(crate) delete_responses: Vec<DeleteResponseCall>,
    #[serde(default)]
    pub(crate) retrieves: Vec<RetrieveCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListCall {
    #[serde(default)]
    pub(crate) purpose: Option<String>,
    #[serde(default)]
    pub(crate) litellm_params: Map<String, Value>,
    #[serde(default)]
    pub(crate) optional_params: Map<String, Value>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) trusted: Option<Vec<(String, WireValue)>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NextCall {
    pub(crate) status: u16,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) litellm_params: Map<String, Value>,
    #[serde(default)]
    pub(crate) optional_params: Map<String, Value>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) trusted: Option<Vec<(String, WireValue)>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListingCall {
    pub(crate) status: u16,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) headers: Vec<(String, String)>,
    #[serde(default)]
    pub(crate) litellm_params: Map<String, Value>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) trusted: Option<Vec<(String, WireValue)>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileCall {
    #[serde(default)]
    pub(crate) file_id: Option<String>,
    #[serde(default)]
    pub(crate) litellm_params: Map<String, Value>,
    #[serde(default)]
    pub(crate) optional_params: Map<String, Value>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) trusted: Option<Vec<(String, WireValue)>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteResponseCall {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) headers: Vec<(String, String)>,
    #[serde(default)]
    pub(crate) additional_args: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrieveCall {
    pub(crate) file_id: String,
    #[serde(default)]
    pub(crate) litellm_params: Map<String, Value>,
    #[serde(default)]
    pub(crate) optional_params: Map<String, Value>,
}

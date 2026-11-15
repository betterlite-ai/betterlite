use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) action: Action,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) bucket: Option<String>,
    pub(crate) object_name: Option<String>,
    pub(crate) payload: Option<Value>,
    pub(crate) dynamic: Option<Value>,
    pub(crate) credentials: Option<String>,
    pub(crate) status: Option<u16>,
    pub(crate) body: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Initialize,
    AsyncHeaders,
    SyncHeaders,
    Upload,
    LoggingConfig,
    VertexInstance,
}

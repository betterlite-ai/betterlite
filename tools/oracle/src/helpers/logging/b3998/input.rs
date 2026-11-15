use std::collections::BTreeMap;

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    #[serde(default)]
    pub(super) bucket: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) payload: Option<StandardLoggingPayload>,
    pub(super) steps: Vec<Step>,
    #[serde(default)]
    pub(super) replies: Vec<Reply>,
    #[serde(default)]
    pub(super) dynamic: Option<Json>,
    #[serde(default)]
    pub(super) request_id: Option<String>,
    #[serde(default)]
    pub(super) start_time: Option<i64>,
    #[serde(default)]
    pub(super) timestamp: Option<i64>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Event,
    Failure,
    Settings,
    Drain,
    Config,
    Group,
    Ndjson,
    Grouped,
    Send,
    Retrieve,
    Date,
    Flush,
    Wait,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    #[serde(rename = "do")]
    pub(super) action: Action,
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) response_id: Option<String>,
    #[serde(default)]
    pub(super) dynamic: Option<Json>,
    #[serde(default)]
    pub(super) metadata: Option<IndexMap<String, Json>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            status: 200,
            body: "{}".to_owned(),
        }
    }
}

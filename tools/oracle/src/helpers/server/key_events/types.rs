use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) event: EventKind,
    pub(super) data: Option<JsonValue>,
    pub(super) response: Option<JsonValue>,
    pub(super) row: Option<JsonValue>,
    #[serde(default)]
    pub(super) rows: Vec<JsonValue>,
    pub(super) caller: Caller,
    pub(super) changed_by: Option<String>,
    pub(super) admin_name: Option<String>,
    pub(super) audit: Option<bool>,
    pub(super) now: String,
    pub(super) id: String,
    pub(super) settings: SinkSettings,
    pub(super) client: bool,
    pub(super) runtime: Runtime,
    #[serde(default)]
    pub(super) teams: BTreeMap<String, JsonData>,
    #[serde(default)]
    pub(super) secrets: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) failing: BTreeSet<String>,
    #[serde(default)]
    pub(super) alerting: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EventKind {
    Generated,
    Updated,
    Rotated,
    Deleted,
}

// UserAPIKeyAuth: api_key and token go into different audit rows, and the metadata allows the header
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Caller {
    pub(super) user_id: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) token: Option<String>,
    pub(super) metadata: Option<JsonValue>,
    pub(super) team_metadata: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SinkSettings {
    pub(super) store_virtual_keys: Option<bool>,
    pub(super) prefix_for_stored_virtual_keys: String,
    pub(super) description: Option<String>,
    pub(super) tags: Option<BTreeMap<String, String>>,
}

// absent is a proxy without a runtime, no_database one without Prisma, ready one that can read teams
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Runtime {
    Absent,
    NoDatabase,
    Ready,
}

pub(super) fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

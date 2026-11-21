use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Team,
    Alias,
    Access,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Flag {
    NoDatabase,
    CacheOnly,
    DbOnly,
    Upsert,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) now: u64,
    pub(crate) fail: Vec<String>,
}

// one lookup run once per step against the same cache, tables and last_db_access_time
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) row: String,
    pub(crate) function: Function,
    pub(crate) target: String,
    pub(crate) flags: Vec<Flag>,
    pub(crate) in_memory_ttl: i32,
    pub(crate) throttled_at: Option<u64>,
    pub(crate) cached_teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    pub(crate) cached_groups: BTreeMap<String, AccessGroupTable>,
    pub(crate) teams: Vec<JsonData>,
    pub(crate) groups: Vec<JsonData>,
    pub(crate) permissions: Vec<JsonData>,
    pub(crate) created: Option<JsonData>,
    pub(crate) steps: Vec<Step>,
}

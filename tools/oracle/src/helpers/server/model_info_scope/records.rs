use std::collections::BTreeMap;
use std::io;

use betterlite_models::model_registration::JsonObject;
use betterlite_server::model_info_scope::error::ScopeError;
use betterlite_server::model_visibility::testing::FixtureError;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Fixture(#[from] FixtureError),
    #[error(transparent)]
    Scope(#[from] ScopeError),
    #[error(transparent)]
    Runtime(#[from] io::Error),
    #[error("the case carries no caller record")]
    MissingCaller,
    #[error("the direct_access function needs a user row")]
    MissingUser,
    #[error("user_teams is \"*\" or a list of team ids, not {0}")]
    UnknownTeams(String),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Predicate {
    NonAdmin,
    TeamModels,
    GrantResolution,
    DirectAccess,
    Populate,
    TeamAndDirect,
    Filter,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRow {
    pub(crate) model_name: String,
    pub(crate) litellm_params: JsonObject,
    #[serde(default)]
    pub(crate) model_info: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserRecord {
    pub(crate) user_id: String,
    #[serde(default)]
    pub(crate) teams: Vec<String>,
    #[serde(default)]
    pub(crate) models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TeamRecord {
    pub(crate) team_id: String,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) access_group_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccessGroupRecord {
    pub(crate) access_group_id: String,
    #[serde(default)]
    pub(crate) access_model_names: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallerRecord {
    #[serde(default)]
    pub(crate) user_id: Option<String>,
    #[serde(default)]
    pub(crate) admin_view: bool,
    #[serde(default)]
    pub(crate) models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum UserTeams {
    Star(String),
    Ids(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Predicate,
    #[serde(default)]
    pub(crate) model_list: Vec<RawRow>,
    #[serde(default)]
    pub(crate) model_group_alias: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) all_models: Vec<RawRow>,
    #[serde(default)]
    pub(crate) grant: Vec<String>,
    #[serde(default)]
    pub(crate) key_models: Vec<String>,
    #[serde(default)]
    pub(crate) user: Option<UserRecord>,
    #[serde(default)]
    pub(crate) creators: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub(crate) teams: Vec<TeamRecord>,
    #[serde(default)]
    pub(crate) user_teams: Option<UserTeams>,
    #[serde(default)]
    pub(crate) access_groups: Vec<AccessGroupRecord>,
    #[serde(default)]
    pub(crate) caller: Option<CallerRecord>,
}

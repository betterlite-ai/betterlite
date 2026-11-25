use std::io;

use betterlite_models::model_registration::JsonObject;
use betterlite_server::model_team_scope::GroupModels;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Runtime(#[from] io::Error),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    ByokRowOutsideCallerTeams,
    TeamModelsResolveToNames,
    CallerByokTeamScope,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallerRecord {
    #[serde(default)]
    pub(crate) user_id: Option<String>,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    #[serde(default)]
    pub(crate) admin_view: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserRecord {
    pub(crate) user_id: String,
    #[serde(default)]
    pub(crate) teams: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) model_info: JsonObject,
    #[serde(default)]
    pub(crate) allowed_team_ids: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) team_models: Vec<String>,
    #[serde(default)]
    pub(crate) access_groups: GroupModels,
    #[serde(default)]
    pub(crate) caller: Option<CallerRecord>,
    #[serde(default)]
    pub(crate) user: Option<UserRecord>,
}

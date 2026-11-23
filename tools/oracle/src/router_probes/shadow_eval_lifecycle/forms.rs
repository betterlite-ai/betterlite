use betterlite_models::model_registration::JsonObject;
use betterlite_server::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use betterlite_server::shadow_eval_models::ShadowEvalTargetType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Payload {
    pub(super) router: Vec<Deployment>,
    pub(super) model_group_alias: JsonObject,
    pub(super) rows: Rows,
    pub(super) jobs: Vec<JobSeed>,
    pub(super) attempts: Vec<AttemptSeed>,
    pub(super) now_millis: i64,
    pub(super) minted: Vec<String>,
    pub(super) seeds_fail: bool,
    pub(super) create_conflict: bool,
    pub(super) steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Deployment {
    pub(super) model_name: String,
    pub(super) litellm_params: JsonObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Rows {
    pub(super) keys: Vec<KeySeed>,
    pub(super) teams: Vec<TeamSeed>,
    pub(super) users: Vec<UserSeed>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct KeySeed {
    pub(super) token: String,
    pub(super) key_alias: Option<String>,
    pub(super) key_name: Option<String>,
    pub(super) team_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct TeamSeed {
    pub(super) team_id: String,
    pub(super) team_alias: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct UserSeed {
    pub(super) user_id: String,
    pub(super) user_email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct JobSeed {
    pub(super) id: String,
    pub(super) group_id: String,
    pub(super) target_type: ShadowEvalTargetType,
    pub(super) target_id: String,
    pub(super) router_name: String,
    #[serde(default)]
    pub(super) router_names: Vec<String>,
    #[serde(default)]
    pub(super) models: Vec<String>,
    pub(super) direction: ShadowEvalDirection,
    #[serde(default)]
    pub(super) baseline_model: Option<String>,
    pub(super) judge_model: String,
    pub(super) shadow_percentage: String,
    pub(super) max_turns: i64,
    pub(super) max_budget: Option<String>,
    pub(super) created_at_millis: i64,
    pub(super) ends_at_millis: i64,
    #[serde(default)]
    pub(super) stopped_at_millis: Option<i64>,
    #[serde(default)]
    pub(super) stopped_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct AttemptSeed {
    pub(super) job_id: String,
    pub(super) created_at_millis: i64,
    pub(super) spend: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Body {
    pub(super) text: String,
    pub(super) pad_to: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Role {
    #[default]
    Absent,
    ProxyAdmin,
    ProxyAdminViewer,
    InternalUser,
    Customer,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct StartStep {
    pub(super) role: Role,
    pub(super) store: bool,
    pub(super) user_id: Option<String>,
    pub(super) body: Body,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct StopStep {
    pub(super) role: Role,
    pub(super) store: bool,
    pub(super) user_id: Option<String>,
    pub(super) job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Step {
    Parse(Body),
    Start(StartStep),
    Stop(StopStep),
}

pub(super) const fn role_name(role: Role) -> Option<&'static str> {
    match role {
        Role::Absent => None,
        Role::ProxyAdmin => Some("proxy_admin"),
        Role::ProxyAdminViewer => Some("proxy_admin_viewer"),
        Role::InternalUser => Some("internal_user"),
        Role::Customer => Some("customer"),
    }
}

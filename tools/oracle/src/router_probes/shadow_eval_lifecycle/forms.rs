mod output;

use betterlite_models::model_registration::JsonObject;
use betterlite_server::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use betterlite_server::shadow_eval_models::ShadowEvalTargetType;
use serde::{Deserialize, Serialize};

pub use self::output::{JobState, JobView, Output, StepOutcome, TargetView};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    pub router: Vec<Deployment>,
    pub model_group_alias: JsonObject,
    pub rows: Rows,
    pub jobs: Vec<JobSeed>,
    pub attempts: Vec<AttemptSeed>,
    pub now_millis: i64,
    pub minted: Vec<String>,
    pub seeds_fail: bool,
    pub create_conflict: bool,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployment {
    pub model_name: String,
    pub litellm_params: JsonObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rows {
    pub keys: Vec<KeySeed>,
    pub teams: Vec<TeamSeed>,
    pub users: Vec<UserSeed>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeySeed {
    pub token: String,
    pub key_alias: Option<String>,
    pub key_name: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamSeed {
    pub team_id: String,
    pub team_alias: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSeed {
    pub user_id: String,
    pub user_email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSeed {
    pub id: String,
    pub group_id: String,
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
    pub router_name: String,
    #[serde(default)]
    pub router_names: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    pub direction: ShadowEvalDirection,
    #[serde(default)]
    pub baseline_model: Option<String>,
    pub judge_model: String,
    pub shadow_percentage: String,
    pub max_turns: i64,
    pub max_budget: Option<String>,
    pub created_at_millis: i64,
    pub ends_at_millis: i64,
    #[serde(default)]
    pub stopped_at_millis: Option<i64>,
    #[serde(default)]
    pub stopped_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptSeed {
    pub job_id: String,
    pub created_at_millis: i64,
    pub spend: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Body {
    pub text: String,
    pub pad_to: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Absent,
    ProxyAdmin,
    ProxyAdminViewer,
    InternalUser,
    Customer,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartStep {
    pub role: Role,
    pub store: bool,
    pub user_id: Option<String>,
    pub body: Body,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StopStep {
    pub role: Role,
    pub store: bool,
    pub user_id: Option<String>,
    pub job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Parse(Body),
    Start(StartStep),
    Stop(StopStep),
}

pub(super) fn role_name(role: Role) -> Option<&'static str> {
    match role {
        Role::Absent => None,
        Role::ProxyAdmin => Some("proxy_admin"),
        Role::ProxyAdminViewer => Some("proxy_admin_viewer"),
        Role::InternalUser => Some("internal_user"),
        Role::Customer => Some("customer"),
    }
}

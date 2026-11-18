use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::member::Member;
use serde::{Deserialize, Serialize};

use crate::key_fields::values::Amount;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkTeamMemberAddRequest {
    pub team_id: String,
    #[serde(default)]
    pub members: Option<Vec<Member>>,
    #[serde(default = "default_all_users")]
    pub all_users: Option<bool>,
    #[serde(default)]
    pub max_budget_in_team: Option<Amount>,
}

const fn default_all_users() -> Option<bool> {
    Some(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMemberAddResult {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    pub updated_user: Option<BTreeMap<String, JsonData>>,
    pub updated_team_membership: Option<BTreeMap<String, JsonData>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkTeamMemberAddResponse {
    pub team_id: String,
    pub results: Vec<TeamMemberAddResult>,
    pub total_requested: i64,
    pub successful_additions: i64,
    pub failed_additions: i64,
    pub updated_team: Option<BTreeMap<String, JsonData>>,
}

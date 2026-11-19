use std::collections::BTreeMap;

use chrono::NaiveDateTime;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::dashboard::payload::JsonPayload;
use crate::finops::model::secret_eq;
use crate::seed::CallerRole;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScimUserRow {
    pub user_id: String,
    #[serde(default)]
    pub created_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub updated_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub user_alias: Option<String>,
    #[serde(default)]
    pub sso_user_id: Option<String>,
    #[serde(default)]
    pub user_role: Option<CallerRole>,
    #[serde(default)]
    pub teams: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, JsonPayload>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScimTeamRow {
    pub team_id: String,
    #[serde(default)]
    pub created_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub updated_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub members_with_roles: Vec<ScimMember>,
    #[serde(default)]
    pub members: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, JsonPayload>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScimMember {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    pub role: MemberRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Admin,
    User,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScimKeyRow {
    pub token: SecretString,
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, JsonPayload>>,
}

impl PartialEq for ScimKeyRow {
    fn eq(&self, other: &Self) -> bool {
        secret_eq(&self.token, &other.token)
            && self.blocked == other.blocked
            && self.metadata == other.metadata
    }
}

impl Eq for ScimKeyRow {}

use std::collections::BTreeSet;

use betterlite_auth::auth_identity::records::Principal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::managed_resource_store::is_proxy_admin;

pub mod source;

mod tests;

pub const NOT_PROXY_ADMIN: &str =
    "Only proxy admins may modify the Claude Code plugin marketplace.";
pub const DB_NOT_CONNECTED: &str = "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys";

#[derive(Debug, Error)]
pub enum ClaudePluginPolicyError {
    #[error("{message}")]
    InvalidSource { message: &'static str },
    #[error("{message}")]
    NotProxyAdmin { message: &'static str },
    #[error("{message}")]
    DatabaseNotConnected { message: &'static str },
    #[error("{message}")]
    Response { status_code: u16, message: String },
    #[error("{message}")]
    Pattern {
        message: &'static str,
        #[source]
        source: regex::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginErrorDetail {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginErrorFields {
    pub status_code: u16,
    pub detail: PluginErrorDetail,
}

impl ClaudePluginPolicyError {
    #[must_use]
    pub fn fields(&self) -> PluginErrorFields {
        let (status_code, message) = match self {
            Self::InvalidSource { message } => (400, *message),
            Self::NotProxyAdmin { message } => (403, *message),
            Self::DatabaseNotConnected { message } => (500, *message),
            Self::Response {
                status_code,
                message,
            } => (*status_code, message.as_str()),
            Self::Pattern { message, .. } => (500, *message),
        };
        PluginErrorFields {
            status_code,
            detail: PluginErrorDetail {
                error: message.to_owned(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginObjectPermission {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct PluginCaller {
    pub principal: Principal,
    pub object_permission: Option<PluginObjectPermission>,
    pub team_object_permission: Option<PluginObjectPermission>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRecord {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillVisibility {
    pub granted: BTreeSet<String>,
    pub sees_private: bool,
}

pub const PUBLIC_ONLY: SkillVisibility = SkillVisibility {
    granted: BTreeSet::new(),
    sees_private: false,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginWhere {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(rename = "OR", skip_serializing_if = "Option::is_none")]
    pub any: Option<Vec<PluginWhereBranch>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum PluginWhereBranch {
    Enabled { enabled: bool },
    Name { name: NameFilter },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameFilter {
    #[serde(rename = "in")]
    pub names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClaudePluginPolicyInput {
    pub caller: Option<PluginCaller>,
    pub plugins: Vec<PluginRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudePluginPolicyResult {
    pub visibility: SkillVisibility,
    pub filter: PluginWhere,
    pub visible: Vec<String>,
}

impl SkillVisibility {
    #[must_use]
    pub fn allows(&self, plugin: &PluginRecord) -> bool {
        if plugin.enabled || self.sees_private {
            return true;
        }
        self.granted.contains(&plugin.name)
    }

    #[must_use]
    pub fn where_clause(&self) -> PluginWhere {
        if self.sees_private {
            return PluginWhere {
                enabled: None,
                any: None,
            };
        }
        if self.granted.is_empty() {
            return PluginWhere {
                enabled: Some(true),
                any: None,
            };
        }
        PluginWhere {
            enabled: None,
            any: Some(vec![
                PluginWhereBranch::Enabled { enabled: true },
                PluginWhereBranch::Name {
                    name: NameFilter {
                        names: self.granted.iter().cloned().collect(),
                    },
                },
            ]),
        }
    }
}

fn skills_of(permission: Option<&PluginObjectPermission>) -> BTreeSet<String> {
    let Some(skills) = permission.and_then(|permission| permission.skills.as_ref()) else {
        return BTreeSet::new();
    };
    skills.iter().cloned().collect()
}

#[must_use]
pub fn granted_skills(caller: &PluginCaller) -> BTreeSet<String> {
    let key = skills_of(caller.object_permission.as_ref());
    let team = skills_of(caller.team_object_permission.as_ref());
    match (key.is_empty(), team.is_empty()) {
        (false, false) => key.intersection(&team).cloned().collect(),
        (false, true) => key,
        _ => team,
    }
}

#[must_use]
pub fn skill_visibility(caller: Option<&PluginCaller>) -> SkillVisibility {
    let Some(caller) = caller else {
        return PUBLIC_ONLY;
    };
    if is_proxy_admin(Some(&caller.principal)) {
        return SkillVisibility {
            granted: BTreeSet::new(),
            sees_private: true,
        };
    }
    SkillVisibility {
        granted: granted_skills(caller),
        sees_private: false,
    }
}

#[must_use]
pub fn name_conflict_error(name: &str) -> ClaudePluginPolicyError {
    ClaudePluginPolicyError::Response {
        status_code: 409,
        message: format!(
            "A skill named '{name}' already exists. Update the existing skill instead of adding it again."
        ),
    }
}

#[must_use]
pub fn plugin_not_found_error(name: &str) -> ClaudePluginPolicyError {
    ClaudePluginPolicyError::Response {
        status_code: 404,
        message: format!("Plugin '{name}' not found"),
    }
}

pub fn require_proxy_admin(caller: Option<&PluginCaller>) -> Result<(), ClaudePluginPolicyError> {
    if caller.is_some_and(|caller| is_proxy_admin(Some(&caller.principal))) {
        return Ok(());
    }
    Err(ClaudePluginPolicyError::NotProxyAdmin {
        message: NOT_PROXY_ADMIN,
    })
}

pub const fn require_database(connected: bool) -> Result<(), ClaudePluginPolicyError> {
    if connected {
        return Ok(());
    }
    Err(ClaudePluginPolicyError::DatabaseNotConnected {
        message: DB_NOT_CONNECTED,
    })
}

#[must_use]
pub fn evaluate(input: &ClaudePluginPolicyInput) -> ClaudePluginPolicyResult {
    let visibility = skill_visibility(input.caller.as_ref());
    let filter = visibility.where_clause();
    let visible = input
        .plugins
        .iter()
        .filter(|plugin| visibility.allows(plugin))
        .map(|plugin| plugin.name.clone())
        .collect();
    ClaudePluginPolicyResult {
        visibility,
        filter,
        visible,
    }
}

use chrono::{DateTime, Utc};
use serde::Deserialize;

pub mod globals;
pub mod visibility;

// LiteLLM stores dashboard session keys under this team id and never lists them
pub const UI_SESSION_TOKEN_TEAM_ID: &str = "litellm-dashboard";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyColumn {
    TeamId,
    UserId,
    OrganizationId,
    KeyAlias,
    Token,
    ProjectId,
    AccessGroupIds,
    AgentId,
    Expires,
    Blocked,
    CreatedBy,
}

impl KeyColumn {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::TeamId => "team_id",
            Self::UserId => "user_id",
            Self::OrganizationId => "organization_id",
            Self::KeyAlias => "key_alias",
            Self::Token => "token",
            Self::ProjectId => "project_id",
            Self::AccessGroupIds => "access_group_ids",
            Self::AgentId => "agent_id",
            Self::Expires => "expires",
            Self::Blocked => "blocked",
            Self::CreatedBy => "created_by",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyScalar {
    Bool(bool),
    Text(String),
}

// one Prisma where entry; the enum fixes the value shape each operator accepts
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Where {
    IsNull(KeyColumn),
    IsNotNull(KeyColumn),
    Equals(KeyColumn, KeyScalar),
    NotEquals(KeyColumn, KeyScalar),
    Before(KeyColumn, DateTime<Utc>),
    AtOrAfter(KeyColumn, DateTime<Utc>),
    ContainsInsensitive(KeyColumn, String),
    In(KeyColumn, Vec<String>),
    HasSome(KeyColumn, Vec<String>),
    And(Vec<WhereClause>),
    Or(Vec<WhereClause>),
}

// a Prisma where object: its entries are ANDed, and an empty one matches every row
pub type WhereClause = Vec<Where>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextMatching {
    #[default]
    Exact,
    Substring,
}

impl Where {
    // the caller picks equals or contains-insensitive for the same column
    #[must_use]
    pub fn matching(column: KeyColumn, matching: TextMatching, value: &str) -> Self {
        match matching {
            TextMatching::Exact => Self::Equals(column, KeyScalar::Text(String::from(value))),
            TextMatching::Substring => Self::ContainsInsensitive(column, String::from(value)),
        }
    }
}

// every field but the two matching modes is one argument of LiteLLM's filter builder
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct KeyFilters {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub organization_id: Option<String>,
    pub key_alias: Option<String>,
    pub key_hash: Option<String>,
    pub exclude_team_id: Option<String>,
    pub admin_team_ids: Option<Vec<String>>,
    pub member_team_ids: Option<Vec<String>>,
    pub include_created_by_keys: bool,
    pub project_id: Option<String>,
    pub access_group_id: Option<String>,
    pub agent_id: Option<String>,
    #[serde(skip)]
    pub user_matching: TextMatching,
    #[serde(skip)]
    pub key_alias_matching: TextMatching,
    pub expires_filter: Option<String>,
    pub search: Option<String>,
    pub status_filter: Option<String>,
}

#[must_use]
pub fn filter_out_ui_session_tokens() -> WhereClause {
    let null_team = Where::IsNull(KeyColumn::TeamId);
    let dashboard = Where::NotEquals(
        KeyColumn::TeamId,
        KeyScalar::Text(String::from(UI_SESSION_TOKEN_TEAM_ID)),
    );
    vec![Where::Or(vec![vec![null_team], vec![dashboard]])]
}

// an exact token hit or a case-insensitive alias substring, never a prefix search
#[must_use]
pub fn build_key_search_where(search: &str) -> WhereClause {
    let token = Where::Equals(KeyColumn::Token, KeyScalar::Text(String::from(search)));
    let alias = Where::ContainsInsensitive(KeyColumn::KeyAlias, String::from(search));
    vec![Where::Or(vec![vec![token], vec![alias]])]
}

#[must_use]
pub fn non_empty(value: &Option<String>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text.as_str()),
        Some(_) | None => None,
    }
}

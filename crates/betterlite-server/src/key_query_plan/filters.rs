use chrono::{DateTime, Utc};

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
    Timestamp(DateTime<Utc>),
}

// one Prisma where entry; the enum fixes the value shape each operator accepts
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyPredicate {
    IsNull(KeyColumn),
    IsNotNull(KeyColumn),
    Equals(KeyColumn, KeyScalar),
    NotEquals(KeyColumn, KeyScalar),
    Before(KeyColumn, DateTime<Utc>),
    AtOrAfter(KeyColumn, DateTime<Utc>),
    ContainsInsensitive(KeyColumn, String),
    In(KeyColumn, Vec<String>),
    HasSome(KeyColumn, Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhereEntry {
    Predicate(KeyPredicate),
    And(Vec<WhereClause>),
    Or(Vec<WhereClause>),
}

// a Prisma where object: its entries are ANDed, and an empty one matches every row
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WhereClause(pub Vec<WhereEntry>);

impl WhereClause {
    #[must_use]
    pub fn single(predicate: KeyPredicate) -> Self {
        Self(vec![WhereEntry::Predicate(predicate)])
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextMatching {
    #[default]
    Exact,
    Substring,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    pub user_matching: TextMatching,
    pub key_alias_matching: TextMatching,
    pub expires_filter: Option<String>,
    pub search: Option<String>,
    pub status_filter: Option<String>,
}

#[must_use]
pub fn filter_out_ui_session_tokens() -> WhereClause {
    WhereClause(vec![WhereEntry::Or(vec![
        WhereClause::single(KeyPredicate::IsNull(KeyColumn::TeamId)),
        WhereClause::single(KeyPredicate::NotEquals(
            KeyColumn::TeamId,
            KeyScalar::Text(String::from(UI_SESSION_TOKEN_TEAM_ID)),
        )),
    ])])
}

#[must_use]
pub fn build_key_search_where(search: &str) -> WhereClause {
    WhereClause(vec![WhereEntry::Or(vec![
        WhereClause::single(KeyPredicate::Equals(
            KeyColumn::Token,
            KeyScalar::Text(String::from(search)),
        )),
        WhereClause::single(KeyPredicate::ContainsInsensitive(
            KeyColumn::KeyAlias,
            String::from(search),
        )),
    ])])
}

#[must_use]
pub fn truthy(value: &Option<String>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text.as_str()),
        Some(_) | None => None,
    }
}

#[must_use]
pub fn matching_predicate(column: KeyColumn, matching: TextMatching, value: &str) -> KeyPredicate {
    match matching {
        TextMatching::Exact => KeyPredicate::Equals(column, KeyScalar::Text(String::from(value))),
        TextMatching::Substring => KeyPredicate::ContainsInsensitive(column, String::from(value)),
    }
}

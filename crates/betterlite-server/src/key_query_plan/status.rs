use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use betterlite_db::identity_rows::key::Expiry;

use crate::key_query_plan::filters::{KeyColumn, KeyPredicate, KeyScalar, WhereClause, WhereEntry};

pub const VALID_EXPIRES_FILTER_VALUES: [&str; 2] = ["active", "expired"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    Active,
    Expired,
    Revoked,
    Deleted,
}

pub const VALID_STATUS_FILTER_VALUES: [KeyStatus; 4] = [
    KeyStatus::Active,
    KeyStatus::Expired,
    KeyStatus::Revoked,
    KeyStatus::Deleted,
];

impl KeyStatus {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyStatusSource {
    pub blocked: Option<bool>,
    pub expires: Option<Expiry>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatusError {
    #[error("{text} is not a timestamp")]
    Expiry { text: String },
}

// revoked wins over expiry, a naive expiry is UTC, and an expiry equal to now is still active
pub fn derive_key_status(
    source: &KeyStatusSource,
    now: DateTime<Utc>,
) -> Result<KeyStatus, StatusError> {
    if source.blocked == Some(true) {
        return Ok(KeyStatus::Revoked);
    }
    let Some(expires) = source.expires.as_ref() else {
        return Ok(KeyStatus::Active);
    };
    let instant = expiry_instant(expires)?;
    Ok(if instant < now {
        KeyStatus::Expired
    } else {
        KeyStatus::Active
    })
}

fn expiry_instant(expiry: &Expiry) -> Result<DateTime<Utc>, StatusError> {
    match expiry {
        Expiry::DateTime(value) => Ok(value.with_timezone(&Utc)),
        Expiry::Text(text) => {
            parse_utc(text).ok_or_else(|| StatusError::Expiry { text: text.clone() })
        }
    }
}

fn parse_utc(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Some(value.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(NaiveDateTime::and_utc)
}

#[must_use]
pub fn build_expires_where_clause(expires_filter: &str, now: DateTime<Utc>) -> WhereClause {
    if expires_filter == "expired" {
        return WhereClause(vec![WhereEntry::And(vec![
            WhereClause::single(KeyPredicate::IsNotNull(KeyColumn::Expires)),
            WhereClause::single(KeyPredicate::Before(KeyColumn::Expires, now)),
        ])]);
    }
    WhereClause(vec![WhereEntry::Or(vec![
        WhereClause::single(KeyPredicate::IsNull(KeyColumn::Expires)),
        WhereClause::single(KeyPredicate::AtOrAfter(KeyColumn::Expires, now)),
    ])])
}

#[must_use]
pub fn not_blocked_where_clause() -> WhereClause {
    WhereClause(vec![WhereEntry::Or(vec![
        WhereClause::single(KeyPredicate::IsNull(KeyColumn::Blocked)),
        WhereClause::single(KeyPredicate::Equals(
            KeyColumn::Blocked,
            KeyScalar::Bool(false),
        )),
    ])])
}

#[must_use]
pub fn build_status_where_clause(
    status_filter: Option<&str>,
    now: DateTime<Utc>,
) -> Option<WhereClause> {
    let filter = status_filter?;
    match filter {
        "revoked" => Some(WhereClause::single(KeyPredicate::Equals(
            KeyColumn::Blocked,
            KeyScalar::Bool(true),
        ))),
        "active" | "expired" => Some(WhereClause(vec![WhereEntry::And(vec![
            not_blocked_where_clause(),
            build_expires_where_clause(filter, now),
        ])])),
        _ => None,
    }
}

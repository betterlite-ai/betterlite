use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use betterlite_db::identity_rows::key::Expiry;

use crate::key_query_plan::filters::{KeyColumn, KeyScalar, Where, WhereClause};

pub const VALID_EXPIRES_FILTER_VALUES: [&str; 2] = ["active", "expired"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    Active,
    Expired,
    Revoked,
    Deleted,
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
    let instant = match expires {
        Expiry::DateTime(value) => value.with_timezone(&Utc),
        Expiry::Text(text) => {
            parse_utc(text).ok_or_else(|| StatusError::Expiry { text: text.clone() })?
        }
    };
    Ok(if instant < now {
        KeyStatus::Expired
    } else {
        KeyStatus::Active
    })
}

fn parse_utc(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Some(value.with_timezone(&Utc));
    }
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    Some(DateTime::from_naive_utc_and_offset(naive, Utc))
}

#[must_use]
pub fn build_expires_where_clause(expires_filter: &str, now: DateTime<Utc>) -> WhereClause {
    if expires_filter == "expired" {
        return vec![Where::And(vec![
            vec![Where::IsNotNull(KeyColumn::Expires)],
            vec![Where::Before(KeyColumn::Expires, now)],
        ])];
    }
    vec![Where::Or(vec![
        vec![Where::IsNull(KeyColumn::Expires)],
        vec![Where::AtOrAfter(KeyColumn::Expires, now)],
    ])]
}

#[must_use]
pub fn not_blocked_where_clause() -> WhereClause {
    vec![Where::Or(vec![
        vec![Where::IsNull(KeyColumn::Blocked)],
        vec![Where::Equals(KeyColumn::Blocked, KeyScalar::Bool(false))],
    ])]
}

#[must_use]
pub fn build_status_where_clause(
    status_filter: Option<&str>,
    now: DateTime<Utc>,
) -> Option<WhereClause> {
    let filter = status_filter?;
    if filter == "revoked" {
        return Some(vec![Where::Equals(
            KeyColumn::Blocked,
            KeyScalar::Bool(true),
        )]);
    }
    if !VALID_EXPIRES_FILTER_VALUES.contains(&filter) {
        return None;
    }
    let expiry = build_expires_where_clause(filter, now);
    Some(vec![Where::And(vec![not_blocked_where_clause(), expiry])])
}

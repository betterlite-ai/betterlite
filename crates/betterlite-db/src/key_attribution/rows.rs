use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::key_digest::is_valid_sha256_hash;
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;
use tokio_postgres::types::FromSql;

use crate::postgres_store::diagnostics::DriverError;

use super::error::KeyAttributionError;
use super::metadata::{KeyMetadata, unanimous};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenDigestRow {
    pub digest: String,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyOwnerRow {
    #[serde(rename = "api_key")]
    pub hashed_api_key: String,
    #[serde(default)]
    pub first_owner: Option<String>,
    #[serde(default)]
    pub last_owner: Option<String>,
}

pub fn token_rows(rows: &[Row]) -> Result<Vec<TokenDigestRow>, KeyAttributionError> {
    rows.iter()
        .map(|row| match token_values(row) {
            Ok(values) => Ok(values),
            Err(source) => Err(KeyAttributionError::TokenDigest { source }),
        })
        .collect()
}

fn token_values(row: &Row) -> Result<TokenDigestRow, DriverError> {
    Ok(TokenDigestRow {
        digest: value(row, "digest")?,
        key_alias: value(row, "key_alias")?,
        team_id: value(row, "team_id")?,
        user_id: value(row, "user_id")?,
    })
}

pub fn owner_rows(rows: &[Row]) -> Result<Vec<DailyOwnerRow>, KeyAttributionError> {
    rows.iter()
        .map(|row| match owner_values(row) {
            Ok(values) => Ok(values),
            Err(source) => Err(KeyAttributionError::DailyOwner { source }),
        })
        .collect()
}

fn owner_values(row: &Row) -> Result<DailyOwnerRow, DriverError> {
    Ok(DailyOwnerRow {
        hashed_api_key: value(row, "api_key")?,
        first_owner: value(row, "first_owner")?,
        last_owner: value(row, "last_owner")?,
    })
}

fn value<T>(row: &Row, column: &str) -> Result<T, DriverError>
where
    T: for<'row> FromSql<'row>,
{
    match row.try_get(column) {
        Ok(value) => Ok(value),
        Err(driver) => Err(DriverError { driver }),
    }
}

#[must_use]
pub fn digest_keys(missing_keys: &BTreeSet<String>) -> BTreeSet<String> {
    missing_keys
        .iter()
        .filter(|key| is_valid_sha256_hash(key))
        .cloned()
        .collect()
}

#[must_use]
pub fn token_metadata(
    rows: &[TokenDigestRow],
    wanted: &BTreeSet<String>,
) -> BTreeMap<String, KeyMetadata> {
    rows.iter()
        .filter(|row| wanted.contains(&row.digest))
        .map(|row| {
            (
                row.digest.clone(),
                KeyMetadata {
                    key_alias: row.key_alias.clone(),
                    team_id: row.team_id.clone(),
                    user_id: row.user_id.clone(),
                    ..KeyMetadata::default()
                },
            )
        })
        .collect()
}

#[must_use]
pub fn unresolved_digests(
    resolved: &BTreeMap<String, KeyMetadata>,
    digests: &BTreeSet<String>,
) -> BTreeSet<String> {
    digests
        .iter()
        .filter(|digest| !resolved.contains_key(*digest))
        .cloned()
        .collect()
}

#[must_use]
pub fn daily_owners(rows: &[DailyOwnerRow], keys: &BTreeSet<String>) -> BTreeMap<String, String> {
    rows.iter()
        .filter(|row| keys.contains(&row.hashed_api_key))
        .filter_map(|row| {
            unanimous(row.first_owner.as_deref(), row.last_owner.as_deref())
                .map(|owner| (row.hashed_api_key.clone(), owner))
        })
        .collect()
}

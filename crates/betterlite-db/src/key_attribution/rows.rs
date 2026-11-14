use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::key_digest::is_valid_sha256_hash;
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;

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
        .map(|row| {
            token_values(row).map_err(|driver| KeyAttributionError::TokenDigest {
                source: DriverError { driver },
            })
        })
        .collect()
}

fn token_values(row: &Row) -> Result<TokenDigestRow, tokio_postgres::Error> {
    Ok(TokenDigestRow {
        digest: row.try_get("digest")?,
        key_alias: row.try_get("key_alias")?,
        team_id: row.try_get("team_id")?,
        user_id: row.try_get("user_id")?,
    })
}

pub fn owner_rows(rows: &[Row]) -> Result<Vec<DailyOwnerRow>, KeyAttributionError> {
    let mut owners = Vec::with_capacity(rows.len());
    for row in rows {
        let owner = owner_values(row).map_err(|driver| KeyAttributionError::DailyOwner {
            source: DriverError { driver },
        })?;
        owners.push(owner);
    }
    Ok(owners)
}

fn owner_values(row: &Row) -> Result<DailyOwnerRow, tokio_postgres::Error> {
    Ok(DailyOwnerRow {
        hashed_api_key: row.try_get("api_key")?,
        first_owner: row.try_get("first_owner")?,
        last_owner: row.try_get("last_owner")?,
    })
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
                    user_email: None,
                    key_exists: None,
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
    let mut owners = BTreeMap::new();
    for row in rows {
        if keys.contains(&row.hashed_api_key)
            && let Some(owner) = unanimous(row.first_owner.as_deref(), row.last_owner.as_deref())
        {
            owners.insert(row.hashed_api_key.clone(), owner);
        }
    }
    owners
}

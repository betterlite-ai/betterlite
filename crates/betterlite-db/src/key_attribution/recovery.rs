use std::collections::{BTreeMap, BTreeSet};

use crate::budget_permission_rows::JsonData;
use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::repository::Repository;
use crate::postgres_store::rows::KeyColumn;
use crate::postgres_store::session::PostgresStore;
use crate::user_repository::tables::USER_TABLE;

use super::bounded::bounded_rows;
use super::error::KeyAttributionError;
use super::metadata::{
    KeyMetadata, UserDetails, attach_details, cli_candidates, cli_metadata, needed_user_ids,
    user_details,
};
use super::projection::{ExportFields, missing_keys, project_rows};
use super::queries::{
    ACTIVE_TOKEN_DIGEST_SQL, DAILY_USER_SPEND_OWNER_SQL, DELETED_TOKEN_DIGEST_SQL,
};
use super::rows::{
    daily_owners, digest_keys, owner_values, token_metadata, token_values, unresolved_digests,
};

pub async fn recover_double_hashed_keys(
    store: &PostgresStore,
    missing_keys: &BTreeSet<String>,
) -> Result<BTreeMap<String, KeyMetadata>, KeyAttributionError> {
    let digests = digest_keys(missing_keys);
    if digests.is_empty() {
        return Ok(BTreeMap::new());
    }
    let from_active = reverse_hash_metadata(store, ACTIVE_TOKEN_DIGEST_SQL, &digests).await?;
    let still_missing = unresolved_digests(&from_active, &digests);
    if still_missing.is_empty() {
        return Ok(from_active);
    }
    let from_deleted =
        reverse_hash_metadata(store, DELETED_TOKEN_DIGEST_SQL, &still_missing).await?;
    Ok(from_active.into_iter().chain(from_deleted).collect())
}

async fn reverse_hash_metadata(
    store: &PostgresStore,
    sql: &str,
    wanted: &BTreeSet<String>,
) -> Result<BTreeMap<String, KeyMetadata>, KeyAttributionError> {
    let values: Vec<String> = wanted.iter().cloned().collect();
    let loaded: Result<Vec<tokio_postgres::Row>, DbError> = async {
        let client = store.session().await?;
        client.query(sql, &[&values]).await.map_err(backend)
    }
    .await;
    match loaded {
        Ok(rows) => {
            let digests = rows
                .iter()
                .map(token_values)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|driver| KeyAttributionError::TokenDigest {
                    source: DriverError { driver },
                })?;
            Ok(token_metadata(&digests, wanted))
        }
        Err(error) => {
            tracing::warn!(
                keys = wanted.len(),
                %error,
                "key attribution token lookup failed; continuing without token evidence"
            );
            Ok(BTreeMap::new())
        }
    }
}

pub async fn recover_daily_spend_owners(
    store: &PostgresStore,
    keys: &BTreeSet<String>,
) -> Result<BTreeMap<String, String>, KeyAttributionError> {
    if keys.is_empty() {
        return Ok(BTreeMap::new());
    }
    let values: Vec<String> = keys.iter().cloned().collect();
    let rows = bounded_rows(store, DAILY_USER_SPEND_OWNER_SQL, &[&values], &[])
        .await
        .inspect_err(|error| {
            tracing::warn!(
                keys = keys.len(),
                %error,
                "daily-spend key owner recovery failed; continuing without owner evidence"
            );
        })
        .unwrap_or_default();
    let owners = rows
        .iter()
        .map(owner_values)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|driver| KeyAttributionError::DailyOwner {
            source: DriverError { driver },
        })?;
    Ok(daily_owners(&owners, keys))
}

pub async fn recover_cli_session_keys(
    store: &PostgresStore,
    missing_keys: &BTreeSet<String>,
) -> Result<BTreeMap<String, KeyMetadata>, KeyAttributionError> {
    let candidates = cli_candidates(missing_keys);
    let user_ids = candidates.values().cloned().collect();
    let details = details_for_user_ids(store, &user_ids).await?;
    Ok(cli_metadata(&candidates, &details))
}

pub async fn attach_user_details(
    store: &PostgresStore,
    mut recovered: BTreeMap<String, KeyMetadata>,
) -> Result<BTreeMap<String, KeyMetadata>, KeyAttributionError> {
    let needing = needed_user_ids(&recovered);
    let details = details_for_user_ids(store, &needing).await?;
    attach_details(&mut recovered, &details);
    Ok(recovered)
}

async fn details_for_user_ids(
    store: &PostgresStore,
    user_ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, UserDetails>, KeyAttributionError> {
    if user_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let client = match store.session().await {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(
                users = user_ids.len(),
                %error,
                "user detail recovery failed; continuing without user evidence"
            );
            return Ok(BTreeMap::new());
        }
    };
    let repository = Repository {
        client: Some(&client),
        table: USER_TABLE,
    };
    let values: Vec<String> = user_ids.iter().cloned().collect();
    match repository.find_many_in(KeyColumn::Id, values).await {
        Ok(users) => Ok(user_details(&users)),
        Err(DbError::InvalidRecord { source }) => Err(KeyAttributionError::User { source }),
        Err(error) => {
            tracing::warn!(
                users = user_ids.len(),
                %error,
                "user detail recovery failed; continuing without user evidence"
            );
            Ok(BTreeMap::new())
        }
    }
}

pub async fn fill_missing_api_key_aliases(
    store: &PostgresStore,
    rows: &[BTreeMap<String, JsonData>],
    fields: &ExportFields,
) -> Result<Vec<BTreeMap<String, JsonData>>, KeyAttributionError> {
    let missing = missing_keys(rows, fields);
    if missing.is_empty() {
        return Ok(rows.to_vec());
    }
    let from_session = recover_cli_session_keys(store, &missing).await?;
    let known: BTreeSet<String> = from_session.keys().cloned().collect();
    let remaining: BTreeSet<String> = missing.difference(&known).cloned().collect();
    let from_double = recover_double_hashed_keys(store, &remaining).await?;
    let merged: BTreeMap<String, KeyMetadata> =
        from_session.into_iter().chain(from_double).collect();
    let recovered = attach_user_details(store, merged).await?;
    if recovered.is_empty() {
        return Ok(rows.to_vec());
    }
    Ok(project_rows(rows, &recovered, fields))
}

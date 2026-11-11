use std::collections::BTreeSet;

use secrecy::{ExposeSecret as _, SecretString};

use crate::database_tls::translation::strip_prisma_query_params;
use crate::migration_ledger::catalog::{max_migration_timestamp, migration_timestamp};
use crate::migration_ledger::connection::connect;

use super::MigrationDriftCheck;

const LEDGER_ROWS_SQL: &str = "SELECT migration_name FROM _prisma_migrations \
    WHERE finished_at IS NOT NULL AND rolled_back_at IS NULL";

// Mirrors _warn_if_db_ahead_of_head: informational only, so every unavailable step returns without a warning
pub async fn warn_if_db_ahead_of_head(
    check: &MigrationDriftCheck,
    known: &[String],
) -> Vec<String> {
    let Some(database_url) = &check.database_url else {
        return Vec::new();
    };
    let Ok(stripped) = strip_prisma_query_params(database_url.expose_secret()) else {
        return Vec::new();
    };
    let Ok(client) = connect(&SecretString::from(stripped)).await else {
        return Vec::new();
    };
    let Ok(rows) = client.query(LEDGER_ROWS_SQL, &[]).await else {
        return Vec::new();
    };
    let mut applied = Vec::with_capacity(rows.len());
    for row in &rows {
        let Ok(name) = row.try_get::<_, String>(0) else {
            return Vec::new();
        };
        applied.push(name);
    }
    let hostile = hostile_names(&applied, known);
    if hostile.is_empty() {
        return hostile;
    }
    tracing::warn!(
        count = hostile.len(),
        migrations = %shown(&hostile),
        "the database has migrations applied that are newer than any migration this build ships; some API endpoints may fail until this deployment is upgraded"
    );
    hostile
}

// Mirrors the applied-minus-known timestamp filter; unknown names without a leading timestamp sort as the oldest
#[must_use]
pub fn hostile_names(applied: &[String], known: &[String]) -> Vec<String> {
    let known_set: BTreeSet<&str> = known.iter().map(String::as_str).collect();
    let head = max_migration_timestamp(known_set.iter().copied());
    let mut hostile: Vec<String> = applied
        .iter()
        .filter(|name| !known_set.contains(name.as_str()) && migration_timestamp(name) > head)
        .cloned()
        .collect();
    hostile.sort();
    hostile.dedup();
    hostile
}

#[must_use]
pub fn shown(hostile: &[String]) -> String {
    let joined: Vec<&str> = hostile.iter().take(5).map(String::as_str).collect();
    if hostile.len() > 5 {
        return format!("{} ...", joined.join(", "));
    }
    joined.join(", ")
}

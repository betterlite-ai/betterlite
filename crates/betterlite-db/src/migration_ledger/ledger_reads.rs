use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Row, types::ToSql};

use super::connection::connect;
use super::environment::{DEFAULT_SCHEMA, prisma_schema_param};
use super::ledger_table;
use crate::database_tls::translation::strip_prisma_query_params;

// The three outcomes ProxyExtrasDBManager._read_migration_ledger distinguishes for its callers
enum LedgerLookup {
    Unavailable,
    Missing,
    Found(Row),
}

// Mirrors _read_migration_ledger: a separate short-lived connection to the stripped libpq URL, and no error escapes
async fn read_migration_ledger(
    database_url: Option<&SecretString>,
    template: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> LedgerLookup {
    let Some(url) = database_url else {
        return LedgerLookup::Unavailable;
    };
    let Ok(schema) = prisma_schema_param(url.expose_secret()) else {
        return LedgerLookup::Unavailable;
    };
    let Ok(stripped) = strip_prisma_query_params(url.expose_secret()) else {
        return LedgerLookup::Unavailable;
    };
    let Ok(client) = connect(&SecretString::from(stripped)).await else {
        return LedgerLookup::Unavailable;
    };
    let table = ledger_table(schema.as_deref().unwrap_or(DEFAULT_SCHEMA));
    match client
        .query_opt(&template.replace("{}", &table), parameters)
        .await
    {
        Ok(Some(row)) => LedgerLookup::Found(row),
        Ok(None) => LedgerLookup::Missing,
        Err(_error) => LedgerLookup::Unavailable,
    }
}

// Mirrors ProxyExtrasDBManager._failed_migration_logs: None when the ledger cannot be read, empty text for null or absent
pub async fn failed_migration_logs(
    database_url: Option<&SecretString>,
    migration_name: &str,
    started_at: &str,
) -> Option<String> {
    let lookup = read_migration_ledger(
        database_url,
        "SELECT logs FROM {} WHERE migration_name = $1 AND started_at = $2::timestamptz \
         AND finished_at IS NULL AND rolled_back_at IS NULL",
        &[&migration_name, &started_at],
    )
    .await;
    match lookup {
        LedgerLookup::Unavailable => None,
        LedgerLookup::Missing => Some(String::new()),
        LedgerLookup::Found(row) => Some(
            row.try_get::<_, Option<String>>(0)
                .ok()
                .flatten()
                .unwrap_or_default(),
        ),
    }
}

// Mirrors ProxyExtrasDBManager._failed_migration_recovered: finished or rolled back with the exact captured identity
pub async fn failed_migration_recovered(
    database_url: Option<&SecretString>,
    migration_name: &str,
    started_at: &str,
) -> bool {
    let lookup = read_migration_ledger(
        database_url,
        "SELECT 1 FROM {} WHERE migration_name = $1 AND started_at = $2::timestamptz \
         AND (finished_at IS NOT NULL OR rolled_back_at IS NOT NULL)",
        &[&migration_name, &started_at],
    )
    .await;
    matches!(lookup, LedgerLookup::Found(_))
}

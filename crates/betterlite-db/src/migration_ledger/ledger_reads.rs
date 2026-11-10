use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Row, types::ToSql};

use super::connection::connect;
use super::environment::{DEFAULT_SCHEMA, prisma_schema_param};
use crate::database_tls::translation::strip_prisma_query_params;

// One probe answers both failed-history reads; the text cast makes the driver bind started_at as text.
const FAILED_MIGRATION_SQL: &str = "SELECT CASE WHEN finished_at IS NULL AND rolled_back_at IS NULL \
     THEN logs END AS failed_logs, \
     (finished_at IS NOT NULL OR rolled_back_at IS NOT NULL) AS recovered \
     FROM {} WHERE migration_name = $1 AND started_at = $2::text::timestamptz";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedMigration {
    pub logs: String,
    pub recovered: bool,
}

#[derive(Debug)]
pub enum FailedLookup {
    Unavailable,
    Absent,
    Found(FailedMigration),
}

// Mirrors _read_migration_ledger: a separate short-lived connection to the stripped libpq URL, and no error escapes
pub async fn look_up_failed_migration(
    database_url: Option<&SecretString>,
    migration_name: &str,
    started_at: &str,
) -> FailedLookup {
    let parameters: [&(dyn ToSql + Sync); 2] = [&migration_name, &started_at];
    let Some(url) = database_url else {
        return FailedLookup::Unavailable;
    };
    let Ok(schema) = prisma_schema_param(url.expose_secret()) else {
        return FailedLookup::Unavailable;
    };
    let Ok(stripped) = strip_prisma_query_params(url.expose_secret()) else {
        return FailedLookup::Unavailable;
    };
    let Ok(client) = connect(&SecretString::from(stripped)).await else {
        return FailedLookup::Unavailable;
    };
    let namespace = schema.as_deref().unwrap_or(DEFAULT_SCHEMA);
    let table = format!(
        "\"{}\".\"_prisma_migrations\"",
        namespace.replace('"', "\"\"")
    );
    match client
        .query_opt(&FAILED_MIGRATION_SQL.replace("{}", &table), &parameters)
        .await
    {
        Ok(Some(row)) => FailedLookup::Found(decode(&row)),
        Ok(None) => FailedLookup::Absent,
        Err(_error) => FailedLookup::Unavailable,
    }
}

// A row whose logs are null or not text reads as empty, and a row answer that cannot decode is not recovered.
fn decode(row: &Row) -> FailedMigration {
    FailedMigration {
        logs: row
            .try_get::<_, Option<String>>(0)
            .ok()
            .flatten()
            .unwrap_or_default(),
        recovered: row.try_get(1).unwrap_or(false),
    }
}

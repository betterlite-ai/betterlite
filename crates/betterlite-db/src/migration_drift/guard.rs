use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio_postgres::Error as PostgresError;

use crate::database_tls::translation::{TranslateError, strip_prisma_query_params};
use crate::database_url_policy::url_codec::UrlSplitError;
use crate::migration_ledger::connection::{ConnectionError, connect};
use crate::migration_ledger::environment::{DEFAULT_SCHEMA, prisma_schema_param};

use super::MigrationDriftCheck;

const LEGACY_LENS_SQL: &str = "SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
    WHERE n.nspname = $1 \
    AND c.relname IN ('LiteLLM_Engine', 'LiteLLM_EngineRun', 'LiteLLM_EngineWorker') LIMIT 1";
const PARTITIONED_SPEND_LOGS_SQL: &str = "SELECT 1 FROM pg_partitioned_table pt \
    JOIN pg_class c ON c.oid = pt.partrelid \
    JOIN pg_namespace n ON n.oid = c.relnamespace \
    WHERE c.relname = 'LiteLLM_SpendLogs' AND n.nspname = $1";

#[derive(Debug, Error)]
pub enum LensGuardError {
    #[error("the database URL could not be parsed for its schema")]
    Schema {
        #[from]
        source: UrlSplitError,
    },
    #[error("the database URL could not be normalized for the Lens rename check")]
    Url(#[from] TranslateError),
    #[error("the database connection for the Lens rename check could not be opened")]
    Connect(#[from] ConnectionError),
    #[error("the database could not be queried for legacy Lens tables")]
    Query(#[source] PostgresError),
    #[error(
        "legacy Lens tables exist, so a schema push would drop saved Lens data; apply the shipped 20261001100000_rename_lens migration to this database schema before retrying"
    )]
    LegacyLensTables,
}

// Mirrors raise_if_lens_rename_pending: guards the Prisma db push path against dropping saved Lens data
pub async fn raise_if_lens_rename_pending(
    check: &MigrationDriftCheck,
) -> Result<(), LensGuardError> {
    let Some(database_url) = &check.database_url else {
        return Ok(());
    };
    let schema = prisma_schema_param(database_url.expose_secret())?
        .unwrap_or_else(|| DEFAULT_SCHEMA.to_owned());
    let stripped = strip_prisma_query_params(database_url.expose_secret())?;
    let client = connect(&SecretString::from(stripped)).await?;
    let legacy = client
        .query_opt(LEGACY_LENS_SQL, &[&schema])
        .await
        .map_err(LensGuardError::Query)?;
    if legacy.is_some() {
        return Err(LensGuardError::LegacyLensTables);
    }
    Ok(())
}

// Mirrors spend_logs_is_partitioned: a driver or database failure reads as "not partitioned"
pub async fn spend_logs_is_partitioned(check: &MigrationDriftCheck) -> bool {
    let Some(database_url) = &check.database_url else {
        return false;
    };
    let Ok(configured) = prisma_schema_param(database_url.expose_secret()) else {
        return false;
    };
    let schema = configured.unwrap_or_else(|| DEFAULT_SCHEMA.to_owned());
    let Ok(stripped) = strip_prisma_query_params(database_url.expose_secret()) else {
        return false;
    };
    let Ok(client) = connect(&SecretString::from(stripped)).await else {
        return false;
    };
    match client
        .query_opt(PARTITIONED_SPEND_LOGS_SQL, &[&schema])
        .await
    {
        Ok(found) => found.is_some(),
        Err(_error) => false,
    }
}

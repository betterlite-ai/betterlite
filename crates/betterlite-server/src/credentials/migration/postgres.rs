use std::time::Duration;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::encryption_scan::report::MigrationReport;
use betterlite_db::encryption_scan::storage::scan_tables;
use betterlite_db::encryption_scan::{DbCipher, MigrationJson, ScanError, decode_migration_json};
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::table_ops::{DEFAULT_ACQUIRE_TIMEOUT, DEFAULT_QUERY_TIMEOUT};
use deadpool_postgres::Client;
use thiserror::Error;
use tokio::time::timeout;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::store::{CallbackRow, CallbackTable, MigrationStore, SettingsRow};

const CONFIG_ROW_SQL: &str =
    "SELECT \"param_value\"::text FROM \"LiteLLM_Config\" WHERE \"param_name\" = $1";
// to_jsonb($2::text) is the string-in-a-Json-column shape Prisma writes for json.dumps(settings).
const SAVE_CONFIG_ROW_SQL: &str = "UPDATE \"LiteLLM_Config\" \
     SET \"param_value\" = to_jsonb($2::text) WHERE \"param_name\" = $1";
const SSO_ROW_SQL: &str =
    "SELECT \"sso_settings\"::text FROM \"LiteLLM_SSOConfig\" WHERE \"id\" = 'sso_config'";
const SAVE_SSO_ROW_SQL: &str = "UPDATE \"LiteLLM_SSOConfig\" \
     SET \"sso_settings\" = to_jsonb($1::text), \"updated_at\" = CURRENT_TIMESTAMP \
     WHERE \"id\" = 'sso_config'";
const TEAM_ROWS_SQL: &str =
    "SELECT \"team_id\", \"metadata\"::text FROM \"LiteLLM_TeamTable\" ORDER BY \"team_id\"";
const SAVE_TEAM_METADATA_SQL: &str = "UPDATE \"LiteLLM_TeamTable\" \
     SET \"metadata\" = to_jsonb($2::text), \"updated_at\" = CURRENT_TIMESTAMP \
     WHERE \"team_id\" = $1";
const TOKEN_ROWS_SQL: &str = "SELECT \"token\", \"metadata\"::text FROM \"LiteLLM_VerificationToken\" \
     ORDER BY \"token\"";
const SAVE_TOKEN_METADATA_SQL: &str = "UPDATE \"LiteLLM_VerificationToken\" \
     SET \"metadata\" = to_jsonb($2::text), \"updated_at\" = CURRENT_TIMESTAMP \
     WHERE \"token\" = $1";

#[derive(Debug, Error)]
pub enum MigrationStoreError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Scan(#[from] ScanError),
    #[error("the {operation} did not finish within {deadline:?}")]
    Timeout {
        operation: &'static str,
        deadline: Duration,
    },
}

// LiteLLM's Prisma awaits carry no deadline of their own; a drained pool or a lock wait fails the call instead of hanging
#[derive(Debug, Clone, Copy)]
pub struct MigrationDeadlines {
    pub acquire: Duration,
    pub statement: Duration,
}

impl Default for MigrationDeadlines {
    fn default() -> Self {
        Self {
            acquire: DEFAULT_ACQUIRE_TIMEOUT,
            statement: DEFAULT_QUERY_TIMEOUT,
        }
    }
}

pub struct PostgresMigrationStore<'db> {
    pub client: PrismaClient<'db>,
    pub deadlines: MigrationDeadlines,
}

impl PostgresMigrationStore<'_> {
    async fn session(&self) -> Result<Client, MigrationStoreError> {
        let acquired = timeout(self.deadlines.acquire, self.client.writer.session()).await;
        match acquired {
            Ok(Ok(session)) => Ok(session),
            Ok(Err(source)) => Err(MigrationStoreError::Database(source)),
            Err(_elapsed) => Err(MigrationStoreError::Timeout {
                operation: "acquire",
                deadline: self.deadlines.acquire,
            }),
        }
    }

    async fn statement(
        &self,
        operation: &'static str,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, MigrationStoreError> {
        let client = self.session().await?;
        let waited = timeout(self.deadlines.statement, client.query(sql, params)).await;
        match waited {
            Ok(Ok(rows)) => Ok(rows),
            Ok(Err(source)) => Err(MigrationStoreError::Database(backend(source))),
            Err(_elapsed) => Err(timed_out(client, operation, self.deadlines.statement)),
        }
    }
}

// A primary-key query returns at most one row, whose first column is the stored JSON text; NULL reads as absent.
fn stored_json(rows: Vec<Row>) -> Result<Option<MigrationJson>, MigrationStoreError> {
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let column = row
        .try_get::<_, Option<String>>(0)
        .map_err(|source| MigrationStoreError::Database(backend(source)))?;
    let Some(text) = column else {
        return Ok(None);
    };
    Ok(Some(decode_migration_json(text.as_bytes())?))
}

impl MigrationStore for PostgresMigrationStore<'_> {
    type Error = MigrationStoreError;

    async fn scan_covered(
        &self,
        cipher: &DbCipher,
    ) -> Result<MigrationReport, MigrationStoreError> {
        let client = self.session().await?;
        let scanned = timeout(self.deadlines.statement, scan_tables(&**client, cipher)).await;
        match scanned {
            Ok(result) => result.map_err(MigrationStoreError::Scan),
            Err(_elapsed) => Err(timed_out(client, "scan_covered", self.deadlines.statement)),
        }
    }

    async fn settings_row(
        &self,
        row: SettingsRow<'_>,
    ) -> Result<Option<MigrationJson>, MigrationStoreError> {
        match row {
            SettingsRow::Config { param_name, .. } => {
                let found = self
                    .statement("settings_row", CONFIG_ROW_SQL, &[&param_name])
                    .await?;
                stored_json(found)
            }
            SettingsRow::Sso => {
                stored_json(self.statement("settings_row", SSO_ROW_SQL, &[]).await?)
            }
        }
    }

    async fn save_settings_row(
        &self,
        row: SettingsRow<'_>,
        value: &MigrationJson,
    ) -> Result<(), MigrationStoreError> {
        let text = serde_json::to_string(value).map_err(ScanError::from)?;
        let sql = save_settings_sql(row);
        match row {
            SettingsRow::Config { param_name, .. } => {
                let _rows = self
                    .statement("save_settings_row", sql, &[&param_name, &text])
                    .await?;
            }
            SettingsRow::Sso => {
                let _rows = self.statement("save_settings_row", sql, &[&text]).await?;
            }
        }
        Ok(())
    }

    async fn callback_rows(
        &self,
        table: CallbackTable,
    ) -> Result<Vec<CallbackRow>, MigrationStoreError> {
        let (read_sql, _save_sql) = callback_sql(table);
        let rows = self.statement("callback_rows", read_sql, &[]).await?;
        let mut found = Vec::with_capacity(rows.len());
        for row in &rows {
            let key = row
                .try_get::<_, String>(0)
                .map_err(|source| MigrationStoreError::Database(backend(source)))?;
            let metadata = row
                .try_get::<_, Option<String>>(1)
                .map_err(|source| MigrationStoreError::Database(backend(source)))?
                .map(|text| decode_migration_json(text.as_bytes()))
                .transpose()?;
            found.push(CallbackRow { key, metadata });
        }
        Ok(found)
    }

    async fn save_callback_metadata(
        &self,
        table: CallbackTable,
        key: &str,
        metadata: &MigrationJson,
    ) -> Result<(), MigrationStoreError> {
        let (_read_sql, save_sql) = callback_sql(table);
        let text = serde_json::to_string(metadata).map_err(ScanError::from)?;
        let _rows = self
            .statement("save_callback_metadata", save_sql, &[&key, &text])
            .await?;
        Ok(())
    }
}

// a statement that overruns its deadline closes its connection, so the server drops the lock wait
fn timed_out(client: Client, operation: &'static str, deadline: Duration) -> MigrationStoreError {
    drop(Client::take(client));
    MigrationStoreError::Timeout {
        operation,
        deadline,
    }
}

const fn callback_sql(table: CallbackTable) -> (&'static str, &'static str) {
    match table {
        CallbackTable::Team => (TEAM_ROWS_SQL, SAVE_TEAM_METADATA_SQL),
        CallbackTable::VerificationToken => (TOKEN_ROWS_SQL, SAVE_TOKEN_METADATA_SQL),
    }
}

const fn save_settings_sql(row: SettingsRow<'_>) -> &'static str {
    match row {
        SettingsRow::Config { .. } => SAVE_CONFIG_ROW_SQL,
        SettingsRow::Sso => SAVE_SSO_ROW_SQL,
    }
}

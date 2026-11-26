use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::encryption_scan::report::MigrationReport;
use betterlite_db::encryption_scan::storage::scan_tables;
use betterlite_db::encryption_scan::{DbCipher, MigrationJson, ScanError, decode_migration_json};
use betterlite_db::postgres_store::errors::{DbError, backend};
use thiserror::Error;
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
     SET \"sso_settings\" = to_jsonb($2::text), \"updated_at\" = CURRENT_TIMESTAMP \
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
}

pub struct PostgresMigrationStore<'db> {
    pub client: PrismaClient<'db>,
}

impl PostgresMigrationStore<'_> {
    async fn read_json(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<MigrationJson>, MigrationStoreError> {
        let client = self.client.writer.session().await?;
        let row = client
            .query_opt(sql, params)
            .await
            .map_err(|source| MigrationStoreError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let column = row
            .try_get::<_, Option<String>>(0)
            .map_err(|source| MigrationStoreError::Database(backend(source)))?;
        let Some(text) = column else {
            return Ok(None);
        };
        let value = decode_migration_json(text.as_bytes())?;
        Ok(Some(value))
    }

    async fn write(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<(), DbError> {
        let client = self.client.writer.session().await?;
        client.execute(sql, params).await.map_err(backend)?;
        Ok(())
    }
}

impl MigrationStore for PostgresMigrationStore<'_> {
    type Error = MigrationStoreError;

    async fn scan_covered(
        &self,
        cipher: &DbCipher,
    ) -> Result<MigrationReport, MigrationStoreError> {
        let client = self.client.writer.session().await?;
        scan_tables(&**client, cipher)
            .await
            .map_err(MigrationStoreError::Scan)
    }

    async fn settings_row(
        &self,
        row: SettingsRow<'_>,
    ) -> Result<Option<MigrationJson>, MigrationStoreError> {
        match row {
            SettingsRow::Config { param_name, .. } => {
                self.read_json(CONFIG_ROW_SQL, &[&param_name]).await
            }
            SettingsRow::Sso => self.read_json(SSO_ROW_SQL, &[]).await,
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
                self.write(sql, &[&param_name, &text]).await?
            }
            SettingsRow::Sso => self.write(sql, &[&text]).await?,
        }
        Ok(())
    }

    async fn callback_rows(
        &self,
        table: CallbackTable,
    ) -> Result<Vec<CallbackRow>, MigrationStoreError> {
        let (read_sql, _save_sql) = callback_sql(table);
        let client = self.client.writer.session().await?;
        let rows = client
            .query(read_sql, &[])
            .await
            .map_err(|source| MigrationStoreError::Database(backend(source)))?;
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
        self.write(save_sql, &[&key, &text]).await?;
        Ok(())
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

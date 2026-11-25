use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::encryption_scan::report::MigrationReport;
use betterlite_db::encryption_scan::storage::scan_tables;
use betterlite_db::encryption_scan::{DbCipher, MigrationJson, ScanError, decode_migration_json};
use betterlite_db::postgres_store::errors::{DbError, backend};
use thiserror::Error;
use tokio_postgres::types::ToSql;

use super::store::{CallbackRow, CallbackTable, MigrationStore};

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
    async fn read_text(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<String>, MigrationStoreError> {
        let client = self.client.writer.session().await?;
        let row = client
            .query_opt(sql, params)
            .await
            .map_err(|source| MigrationStoreError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        row.try_get::<_, Option<String>>(0)
            .map_err(|source| MigrationStoreError::Database(backend(source)))
    }

    async fn read_json(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<MigrationJson>, MigrationStoreError> {
        let Some(text) = self.read_text(sql, params).await? else {
            return Ok(None);
        };
        let value = decode_migration_json(text.as_bytes())?;
        Ok(Some(value))
    }

    async fn write(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<(), MigrationStoreError> {
        let client = self.client.writer.session().await?;
        client
            .execute(sql, params)
            .await
            .map_err(|source| MigrationStoreError::Database(backend(source)))?;
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

    async fn config_row(
        &self,
        param_name: &str,
    ) -> Result<Option<MigrationJson>, MigrationStoreError> {
        self.read_json(CONFIG_ROW_SQL, &[param_name]).await
    }

    async fn save_config_row(
        &self,
        param_name: &str,
        value: &MigrationJson,
    ) -> Result<(), MigrationStoreError> {
        let text = json_text(value)?;
        self.write(SAVE_CONFIG_ROW_SQL, &[&param_name, &text]).await
    }

    async fn sso_row(&self) -> Result<Option<MigrationJson>, MigrationStoreError> {
        self.read_json(SSO_ROW_SQL, &[]).await
    }

    async fn save_sso_row(&self, value: &MigrationJson) -> Result<(), MigrationStoreError> {
        let text = json_text(value)?;
        self.write(SAVE_SSO_ROW_SQL, &[&text]).await
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
        let text = json_text(metadata)?;
        self.write(save_sql, &[&key, &text]).await
    }
}

const fn callback_sql(table: CallbackTable) -> (&'static str, &'static str) {
    match table {
        CallbackTable::Team => (TEAM_ROWS_SQL, SAVE_TEAM_METADATA_SQL),
        CallbackTable::VerificationToken => (TOKEN_ROWS_SQL, SAVE_TOKEN_METADATA_SQL),
    }
}

fn json_text(value: &MigrationJson) -> Result<String, MigrationStoreError> {
    let text = serde_json::to_string(value).map_err(ScanError::from)?;
    Ok(text)
}

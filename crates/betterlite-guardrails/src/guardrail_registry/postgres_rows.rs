use std::sync::Arc;

use betterlite_db::auxiliary_repository::bindings::GuardrailsTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use betterlite_db::postgres_store::errors::backend;
use betterlite_redis::config_sync::publish::PublishChange;
use tokio_postgres::types::ToSql;

use super::rows::{GuardrailRow, GuardrailRows, GuardrailUpdate, NewGuardrailRow, RowsError};

pub(crate) mod sql;

pub struct PostgresGuardrailRows<'db> {
    base: PrismaTableRepository<'db, GuardrailsTable>,
    publish: PublishChange<TableError>,
}

impl<'db> PostgresGuardrailRows<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, Arc::clone(&publish), GuardrailsTable),
            publish,
        }
    }

    async fn read_one(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<GuardrailRow>, RowsError> {
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let row = client
            .query_opt(sql, params)
            .await
            .map_err(|source| RowsError::Database(backend(source)))?;
        row.as_ref().map(decode_row).transpose()
    }

    async fn read_many(&self, sql: &str) -> Result<Vec<GuardrailRow>, RowsError> {
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let rows = client
            .query(sql, &[])
            .await
            .map_err(|source| RowsError::Database(backend(source)))?;
        rows.iter().map(decode_row).collect()
    }

    // B726 has no guardrails adapter, so writes run here and publish as the config-sync wrapper does.
    async fn write_one(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<GuardrailRow>, RowsError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        let row = client
            .query_opt(sql, params)
            .await
            .map_err(|source| RowsError::Database(backend(source)))?;
        (self.publish)(GuardrailsTable::NAME).await?;
        row.as_ref().map(decode_row).transpose()
    }
}

fn decode_row(row: &tokio_postgres::Row) -> Result<GuardrailRow, RowsError> {
    let text: String = row
        .try_get(0)
        .map_err(|source| RowsError::Database(backend(source)))?;
    serde_json::from_str(&text).map_err(|source| RowsError::Row { source })
}

impl GuardrailRows for PostgresGuardrailRows<'_> {
    async fn create(&self, row: NewGuardrailRow) -> Result<GuardrailRow, RowsError> {
        let statement = sql::insert();
        let created_at = row.created_at.naive_utc();
        let updated_at = row.updated_at.naive_utc();
        self.write_one(
            &statement,
            &[
                &row.guardrail_id,
                &row.guardrail_name,
                &row.litellm_params,
                &row.guardrail_info,
                &created_at,
                &updated_at,
            ],
        )
        .await?
        .ok_or(RowsError::MissingRow)
    }

    async fn delete(&self, guardrail_id: &str) -> Result<(), RowsError> {
        let statement = sql::delete();
        let deleted = self.write_one(&statement, &[&guardrail_id]).await?;
        if deleted.is_none() {
            return Err(RowsError::MissingRow);
        }
        Ok(())
    }

    async fn update(
        &self,
        guardrail_id: &str,
        row: GuardrailUpdate,
    ) -> Result<Option<GuardrailRow>, RowsError> {
        let statement = sql::update();
        let updated_at = row.updated_at.naive_utc();
        self.write_one(
            &statement,
            &[
                &guardrail_id,
                &row.guardrail_name,
                &row.litellm_params,
                &row.guardrail_info,
                &updated_at,
            ],
        )
        .await
    }

    async fn find_active(&self) -> Result<Vec<GuardrailRow>, RowsError> {
        let statement = sql::select_active();
        self.read_many(&statement).await
    }

    async fn find_by_id(&self, guardrail_id: &str) -> Result<Option<GuardrailRow>, RowsError> {
        let statement = sql::select_by_id();
        self.read_one(&statement, &[&guardrail_id]).await
    }

    async fn find_by_name(&self, guardrail_name: &str) -> Result<Option<GuardrailRow>, RowsError> {
        let statement = sql::select_by_name();
        self.read_one(&statement, &[&guardrail_name]).await
    }
}

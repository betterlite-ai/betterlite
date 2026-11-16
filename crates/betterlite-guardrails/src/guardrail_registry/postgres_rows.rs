use std::sync::Arc;

use betterlite_db::auxiliary_repository::bindings::GuardrailsTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding as _, TableError,
};
use betterlite_db::postgres_store::errors::backend;
use betterlite_redis::config_sync::publish::PublishChange;
use tokio_postgres::types::ToSql;

use super::rows::{GuardrailRow, GuardrailRows, GuardrailUpdate, NewGuardrailRow, RowsError};

// LiteLLM stores both Json columns as text, so each one is projected as its text value.
macro_rules! projection {
    () => {
        concat!(
            "jsonb_build_object(",
            "'guardrail_id', guardrail_id, ",
            "'guardrail_name', guardrail_name, ",
            "'litellm_params', litellm_params::text, ",
            "'guardrail_info', guardrail_info::text, ",
            "'team_id', team_id, ",
            "'status', status, ",
            "'created_at', created_at, ",
            "'updated_at', updated_at",
            ")::text"
        )
    };
}

pub(super) const SELECT_ACTIVE: &str = concat!(
    "SELECT ",
    projection!(),
    " FROM \"LiteLLM_GuardrailsTable\" WHERE status = 'active' ORDER BY created_at DESC"
);

pub(super) const SELECT_BY_ID: &str = concat!(
    "SELECT ",
    projection!(),
    " FROM \"LiteLLM_GuardrailsTable\" WHERE guardrail_id = $1"
);

pub(super) const SELECT_BY_NAME: &str = concat!(
    "SELECT ",
    projection!(),
    " FROM \"LiteLLM_GuardrailsTable\" WHERE guardrail_name = $1"
);

pub(super) const INSERT: &str = concat!(
    "INSERT INTO \"LiteLLM_GuardrailsTable\" ",
    "(guardrail_id, guardrail_name, litellm_params, guardrail_info, created_at, updated_at) ",
    "VALUES ($1, $2, to_jsonb($3::text), to_jsonb($4::text), $5, $6) RETURNING ",
    projection!()
);

pub(super) const UPDATE: &str = concat!(
    "UPDATE \"LiteLLM_GuardrailsTable\" SET guardrail_name = $2, ",
    "litellm_params = to_jsonb($3::text), guardrail_info = to_jsonb($4::text), updated_at = $5 ",
    "WHERE guardrail_id = $1 RETURNING ",
    projection!()
);

pub(super) const DELETE: &str = concat!(
    "DELETE FROM \"LiteLLM_GuardrailsTable\" WHERE guardrail_id = $1 RETURNING ",
    projection!()
);

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

    // B726 has no guardrails adapter, so a write runs here and publishes as the config-sync wrapper does.
    async fn fetch(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        write: bool,
    ) -> Result<Vec<GuardrailRow>, RowsError> {
        let access = self.base.access()?;
        let store = if write { access.write } else { access.read };
        let client = store.session().await?;
        let rows = client
            .query(sql, params)
            .await
            .map_err(|source| RowsError::Database(backend(source)))?;
        if write {
            (self.publish)(GuardrailsTable::NAME).await?;
        }
        rows.iter().map(decode_row).collect()
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
        let created_at = row.created_at.naive_utc();
        let updated_at = row.fields.updated_at.naive_utc();
        let params: [&(dyn ToSql + Sync); 6] = [
            &row.guardrail_id,
            &row.fields.guardrail_name,
            &row.fields.litellm_params,
            &row.fields.guardrail_info,
            &created_at,
            &updated_at,
        ];
        let rows = self.fetch(INSERT, &params, true).await?;
        rows.into_iter().next().ok_or(RowsError::MissingRow)
    }

    async fn delete(&self, guardrail_id: &str) -> Result<(), RowsError> {
        let rows = self.fetch(DELETE, &[&guardrail_id], true).await?;
        if rows.is_empty() {
            return Err(RowsError::MissingRow);
        }
        Ok(())
    }

    async fn update(
        &self,
        guardrail_id: &str,
        row: GuardrailUpdate,
    ) -> Result<Option<GuardrailRow>, RowsError> {
        let updated_at = row.updated_at.naive_utc();
        let params: [&(dyn ToSql + Sync); 5] = [
            &guardrail_id,
            &row.guardrail_name,
            &row.litellm_params,
            &row.guardrail_info,
            &updated_at,
        ];
        let rows = self.fetch(UPDATE, &params, true).await?;
        Ok(rows.into_iter().next())
    }

    async fn find_active(&self) -> Result<Vec<GuardrailRow>, RowsError> {
        self.fetch(SELECT_ACTIVE, &[], false).await
    }

    async fn find_by_id(&self, guardrail_id: &str) -> Result<Option<GuardrailRow>, RowsError> {
        let rows = self.fetch(SELECT_BY_ID, &[&guardrail_id], false).await?;
        Ok(rows.into_iter().next())
    }

    async fn find_by_name(&self, guardrail_name: &str) -> Result<Option<GuardrailRow>, RowsError> {
        let rows = self
            .fetch(SELECT_BY_NAME, &[&guardrail_name], false)
            .await?;
        Ok(rows.into_iter().next())
    }
}

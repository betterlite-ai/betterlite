use std::sync::Arc;

use betterlite_redis::config_sync::publish::PublishChange;
use chrono::NaiveDateTime;
use deadpool_postgres::GenericClient as _;
use tokio_postgres::Row;
use tokio_postgres::types::{Json, ToSql};

use crate::auxiliary_repository::bindings::SearchToolsTable;
use crate::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding as _, TableError,
};
use crate::mcp_stored_contracts::JsonValue;
use crate::postgres_store::errors::backend;

use super::super::record::SearchToolRecord;
use super::{CreatedAtOrder, SearchToolCreate, SearchToolTableClient, SearchToolUpdate};

const CREATE_SQL: &str = "INSERT INTO \"LiteLLM_SearchToolsTable\" \
     (search_tool_id, search_tool_name, litellm_params, search_tool_info, created_at, updated_at) \
     VALUES (gen_random_uuid()::text, $1, $2::text::jsonb, $3::text::jsonb, $4, $5) \
     RETURNING search_tool_id, search_tool_name, litellm_params, search_tool_info, created_at, updated_at";

const FIND_SQL: &str = "SELECT search_tool_id, search_tool_name, litellm_params, search_tool_info, \
     created_at, updated_at FROM \"LiteLLM_SearchToolsTable\" WHERE search_tool_id = $1";

const LIST_SQL: &str = "SELECT search_tool_id, search_tool_name, litellm_params, search_tool_info, \
     created_at, updated_at FROM \"LiteLLM_SearchToolsTable\"";

const LIST_ASCENDING_SQL: &str = "SELECT search_tool_id, search_tool_name, litellm_params, \
     search_tool_info, created_at, updated_at FROM \"LiteLLM_SearchToolsTable\" \
     ORDER BY created_at ASC";

const LIST_DESCENDING_SQL: &str = "SELECT search_tool_id, search_tool_name, litellm_params, \
     search_tool_info, created_at, updated_at FROM \"LiteLLM_SearchToolsTable\" \
     ORDER BY created_at DESC";

const UPDATE_SQL: &str = "UPDATE \"LiteLLM_SearchToolsTable\" SET search_tool_name = $2, \
     litellm_params = $3::text::jsonb, search_tool_info = $4::text::jsonb, updated_at = $5 \
     WHERE search_tool_id = $1 \
     RETURNING search_tool_id, search_tool_name, litellm_params, search_tool_info, created_at, updated_at";

const DELETE_SQL: &str = "DELETE FROM \"LiteLLM_SearchToolsTable\" WHERE search_tool_id = $1 \
     RETURNING search_tool_id, search_tool_name, litellm_params, search_tool_info, created_at, updated_at";

type Params<'query> = [&'query (dyn ToSql + Sync)];

pub struct PostgresSearchTools<'db> {
    base: PrismaTableRepository<'db, SearchToolsTable>,
    publish: PublishChange<TableError>,
}

impl<'db> PostgresSearchTools<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, Arc::clone(&publish), SearchToolsTable),
            publish,
        }
    }

    // A read borrows the pool's reader session and reports a backend failure the same way
    async fn read_rows(&self, sql: &str, params: &Params<'_>) -> Result<Vec<Row>, TableError> {
        let client = self.base.access()?.read.session().await?;
        client
            .query(sql, params)
            .await
            .map_err(|source| TableError::Database(backend(source)))
    }

    // A write takes the writer session, and a statement that returns no row names a missing row
    async fn write_row(&self, sql: &str, params: &Params<'_>) -> Result<Row, TableError> {
        let client = self.base.access()?.write.session().await?;
        client
            .query_opt(sql, params)
            .await
            .map_err(|source| TableError::Database(backend(source)))?
            .ok_or(TableError::MissingRow)
    }

    // Config-sync listeners hear about a table change only after the row is written
    async fn published(&self, row: &Row) -> Result<SearchToolRecord, TableError> {
        let record = decode(row)?;
        (self.publish)(SearchToolsTable::NAME).await?;
        Ok(record)
    }
}

fn decode(row: &Row) -> Result<SearchToolRecord, TableError> {
    let search_tool_id = row
        .try_get("search_tool_id")
        .map_err(|source| TableError::Database(backend(source)))?;
    let search_tool_name = row
        .try_get("search_tool_name")
        .map_err(|source| TableError::Database(backend(source)))?;
    let litellm_params: Json<JsonValue> = row
        .try_get("litellm_params")
        .map_err(|source| TableError::Database(backend(source)))?;
    let search_tool_info: Option<Json<JsonValue>> = row
        .try_get("search_tool_info")
        .map_err(|source| TableError::Database(backend(source)))?;
    let created_at: NaiveDateTime = row
        .try_get("created_at")
        .map_err(|source| TableError::Database(backend(source)))?;
    let updated_at: NaiveDateTime = row
        .try_get("updated_at")
        .map_err(|source| TableError::Database(backend(source)))?;
    Ok(SearchToolRecord {
        search_tool_id,
        search_tool_name,
        litellm_params: litellm_params.0,
        search_tool_info: search_tool_info.map(|stored| stored.0),
        created_at: created_at.and_utc(),
        updated_at: updated_at.and_utc(),
    })
}

impl SearchToolTableClient for PostgresSearchTools<'_> {
    async fn create(&self, data: SearchToolCreate) -> Result<SearchToolRecord, TableError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        let created_at = data.created_at.naive_utc();
        let updated_at = data.updated_at.naive_utc();
        let row = client
            .query_one(
                CREATE_SQL,
                &[
                    &data.search_tool_name,
                    &data.litellm_params,
                    &data.search_tool_info,
                    &created_at,
                    &updated_at,
                ],
            )
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        self.published(&row).await
    }

    async fn find_unique(
        &self,
        search_tool_id: &str,
    ) -> Result<Option<SearchToolRecord>, TableError> {
        let mut rows = self.read_rows(FIND_SQL, &[&search_tool_id]).await?;
        rows.pop().map(|row| decode(&row)).transpose()
    }

    async fn find_many(
        &self,
        order: Option<CreatedAtOrder>,
    ) -> Result<Vec<SearchToolRecord>, TableError> {
        let sql = match order {
            None => LIST_SQL,
            Some(CreatedAtOrder::Ascending) => LIST_ASCENDING_SQL,
            Some(CreatedAtOrder::Descending) => LIST_DESCENDING_SQL,
        };
        let rows = self.read_rows(sql, &[]).await?;
        rows.iter().map(decode).collect()
    }

    async fn update(
        &self,
        search_tool_id: &str,
        data: SearchToolUpdate,
    ) -> Result<SearchToolRecord, TableError> {
        let updated_at = data.updated_at.naive_utc();
        let row = self
            .write_row(
                UPDATE_SQL,
                &[
                    &search_tool_id,
                    &data.search_tool_name,
                    &data.litellm_params,
                    &data.search_tool_info,
                    &updated_at,
                ],
            )
            .await?;
        self.published(&row).await
    }

    async fn delete(&self, search_tool_id: &str) -> Result<SearchToolRecord, TableError> {
        let row = self.write_row(DELETE_SQL, &[&search_tool_id]).await?;
        self.published(&row).await
    }
}

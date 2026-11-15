use std::sync::Arc;

use betterlite_redis::config_sync::publish::PublishChange;
use chrono::NaiveDateTime;
use deadpool_postgres::GenericClient as _;
use tokio_postgres::Row;
use tokio_postgres::types::Json;

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
     VALUES (gen_random_uuid()::text, $1, to_jsonb($2::text), to_jsonb($3::text), $4, $5) \
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
     litellm_params = to_jsonb($3::text), search_tool_info = to_jsonb($4::text), updated_at = $5 \
     WHERE search_tool_id = $1 \
     RETURNING search_tool_id, search_tool_name, litellm_params, search_tool_info, created_at, updated_at";

const DELETE_SQL: &str = "DELETE FROM \"LiteLLM_SearchToolsTable\" WHERE search_tool_id = $1 \
     RETURNING search_tool_id, search_tool_name, litellm_params, search_tool_info, created_at, updated_at";

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
        let record = decode(&row)?;
        (self.publish)(SearchToolsTable::NAME).await?;
        Ok(record)
    }

    async fn find_unique(
        &self,
        search_tool_id: &str,
    ) -> Result<Option<SearchToolRecord>, TableError> {
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let row = client
            .query_opt(FIND_SQL, &[&search_tool_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        row.as_ref().map(decode).transpose()
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
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let rows = client
            .query(sql, &[])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        rows.iter().map(decode).collect()
    }

    async fn update(
        &self,
        search_tool_id: &str,
        data: SearchToolUpdate,
    ) -> Result<SearchToolRecord, TableError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        let updated_at = data.updated_at.naive_utc();
        let row = client
            .query_opt(
                UPDATE_SQL,
                &[
                    &search_tool_id,
                    &data.search_tool_name,
                    &data.litellm_params,
                    &data.search_tool_info,
                    &updated_at,
                ],
            )
            .await
            .map_err(|source| TableError::Database(backend(source)))?
            .ok_or(TableError::MissingRow)?;
        let record = decode(&row)?;
        (self.publish)(SearchToolsTable::NAME).await?;
        Ok(record)
    }

    async fn delete(&self, search_tool_id: &str) -> Result<SearchToolRecord, TableError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        let row = client
            .query_opt(DELETE_SQL, &[&search_tool_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?
            .ok_or(TableError::MissingRow)?;
        let record = decode(&row)?;
        (self.publish)(SearchToolsTable::NAME).await?;
        Ok(record)
    }
}

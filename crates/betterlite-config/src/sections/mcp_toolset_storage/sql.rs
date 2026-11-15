use std::sync::Arc;

use betterlite_db::auxiliary_repository::bindings::MCPToolsetTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_redis::config_sync::publish::PublishChange;
use chrono::NaiveDateTime;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::MCPToolsetStorageError;
use super::table::{Patch, ToolsetFilter, ToolsetRow, ToolsetTable, ToolsetUpdate};

const CREATE_SQL: &str = "INSERT INTO \"LiteLLM_MCPToolsetTable\" (toolset_id, toolset_name, description, tools, created_by, updated_by) VALUES ($1, $2, $3, to_jsonb($4::text), $5, $6) RETURNING toolset_id, toolset_name, description, tools #>> '{}'::text[], created_at, created_by, updated_at, updated_by";

const SELECT_BY_ID_SQL: &str = "SELECT toolset_id, toolset_name, description, tools #>> '{}'::text[], created_at, created_by, updated_at, updated_by FROM \"LiteLLM_MCPToolsetTable\" WHERE toolset_id = $1";

const SELECT_BY_NAME_SQL: &str = "SELECT toolset_id, toolset_name, description, tools #>> '{}'::text[], created_at, created_by, updated_at, updated_by FROM \"LiteLLM_MCPToolsetTable\" WHERE toolset_name = $1 LIMIT 1";

const SELECT_ALL_SQL: &str = "SELECT toolset_id, toolset_name, description, tools #>> '{}'::text[], created_at, created_by, updated_at, updated_by FROM \"LiteLLM_MCPToolsetTable\"";

const SELECT_IDS_SQL: &str = "SELECT toolset_id, toolset_name, description, tools #>> '{}'::text[], created_at, created_by, updated_at, updated_by FROM \"LiteLLM_MCPToolsetTable\" WHERE toolset_id = ANY($1::text[])";

const UPDATE_SQL: &str = "UPDATE \"LiteLLM_MCPToolsetTable\" SET toolset_name = COALESCE($2::text, toolset_name), description = CASE WHEN $3::boolean THEN $4::text ELSE description END, tools = COALESCE(to_jsonb($5::text), tools), updated_by = $6, updated_at = now() WHERE toolset_id = $1 RETURNING toolset_id, toolset_name, description, tools #>> '{}'::text[], created_at, created_by, updated_at, updated_by";

const DELETE_SQL: &str = "DELETE FROM \"LiteLLM_MCPToolsetTable\" WHERE toolset_id = $1 RETURNING toolset_id, toolset_name, description, tools #>> '{}'::text[], created_at, created_by, updated_at, updated_by";

pub struct MCPToolsetRepository<'db> {
    pub client: Option<PrismaClient<'db>>,
    pub publish: PublishChange<TableError>,
}

impl<'db> MCPToolsetRepository<'db> {
    fn table(&self, use_writer: bool) -> PrismaTableRepository<'db, MCPToolsetTable> {
        let base =
            PrismaTableRepository::new(self.client, Arc::clone(&self.publish), MCPToolsetTable);
        if use_writer { base.with_writer() } else { base }
    }

    async fn lookup(
        &self,
        sql: &str,
        param: &str,
    ) -> Result<Option<ToolsetRow>, MCPToolsetStorageError> {
        let client = self.table(false).access()?.read.session().await?;
        let found = client.query_opt(sql, &[&param]).await?;
        found_row(found)
    }
}

impl From<DbError> for MCPToolsetStorageError {
    fn from(source: DbError) -> Self {
        Self::Table(TableError::Database(source))
    }
}

impl From<tokio_postgres::Error> for MCPToolsetStorageError {
    fn from(source: tokio_postgres::Error) -> Self {
        Self::Table(TableError::Database(backend(source)))
    }
}

impl ToolsetTable for MCPToolsetRepository<'_> {
    async fn create(&self, row: ToolsetRow) -> Result<ToolsetRow, MCPToolsetStorageError> {
        let client = self.table(false).access()?.write.session().await?;
        let params: [&(dyn ToSql + Sync); 6] = [
            &row.toolset_id,
            &row.toolset_name,
            &row.description,
            &row.tools,
            &row.created_by,
            &row.updated_by,
        ];
        client.query_opt(CREATE_SQL, &params).await?.map_or_else(
            || Err(MCPToolsetStorageError::Table(TableError::MissingRow)),
            |stored| toolset_row(&stored),
        )
    }

    async fn find_unique(
        &self,
        toolset_id: &str,
    ) -> Result<Option<ToolsetRow>, MCPToolsetStorageError> {
        self.lookup(SELECT_BY_ID_SQL, toolset_id).await
    }

    async fn find_first_by_name(
        &self,
        toolset_name: &str,
    ) -> Result<Option<ToolsetRow>, MCPToolsetStorageError> {
        self.lookup(SELECT_BY_NAME_SQL, toolset_name).await
    }

    async fn find_many(
        &self,
        filter: ToolsetFilter,
        use_writer: bool,
    ) -> Result<Vec<ToolsetRow>, MCPToolsetStorageError> {
        let client = self.table(use_writer).access()?.read.session().await?;
        let rows = match filter {
            ToolsetFilter::All => client.query(SELECT_ALL_SQL, &[]).await?,
            ToolsetFilter::Ids(ids) => client.query(SELECT_IDS_SQL, &[&ids]).await?,
        };
        rows.iter().map(toolset_row).collect()
    }

    async fn update(
        &self,
        toolset_id: &str,
        update: ToolsetUpdate,
    ) -> Result<Option<ToolsetRow>, MCPToolsetStorageError> {
        let client = self.table(false).access()?.write.session().await?;
        let (description_set, description): (bool, Option<&str>) = match &update.description {
            Patch::Keep => (false, None),
            Patch::Clear => (true, None),
            Patch::Set(text) => (true, Some(text.as_str())),
        };
        let params: [&(dyn ToSql + Sync); 6] = [
            &toolset_id,
            &update.toolset_name,
            &description_set,
            &description,
            &update.tools,
            &update.updated_by,
        ];
        let stored = client.query_opt(UPDATE_SQL, &params).await?;
        found_row(stored)
    }

    async fn delete(&self, toolset_id: &str) -> Result<Option<ToolsetRow>, MCPToolsetStorageError> {
        let client = self.table(false).access()?.write.session().await?;
        found_row(client.query_opt(DELETE_SQL, &[&toolset_id]).await?)
    }
}

fn found_row(row: Option<Row>) -> Result<Option<ToolsetRow>, MCPToolsetStorageError> {
    let Some(row) = row else {
        return Ok(None);
    };
    toolset_row(&row).map(Some)
}

fn toolset_row(row: &Row) -> Result<ToolsetRow, MCPToolsetStorageError> {
    Ok(ToolsetRow {
        toolset_id: row.try_get(0)?,
        toolset_name: row.try_get(1)?,
        description: row.try_get(2)?,
        tools: row.try_get(3)?,
        created_at: row.try_get::<_, NaiveDateTime>(4).map(Some)?,
        created_by: row.try_get(5)?,
        updated_at: row.try_get::<_, NaiveDateTime>(6).map(Some)?,
        updated_by: row.try_get(7)?,
    })
}

use std::convert::Infallible;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_rows::model::ProxyModelTable;
use tokio_postgres::types::ToSql;

use super::search::{ModelFilter, SearchDatabase, SearchDatabaseError};

// Prisma's contains reaches Postgres unescaped, so % _ and \ stay LIKE metacharacters and equals is a plain equality.
const COUNT_BY_NAME_SQL: &str = r#"SELECT COUNT(*)::BIGINT FROM "LiteLLM_ProxyModelTable" entry
    WHERE entry.model_name = $1 AND entry.model_id <> ALL($2::text[])"#;
const COUNT_BY_CONTAINS_SQL: &str = r#"SELECT COUNT(*)::BIGINT FROM "LiteLLM_ProxyModelTable" entry
    WHERE entry.model_name ILIKE $1 AND entry.model_id <> ALL($2::text[])"#;
const FIND_BY_NAME_SQL: &str = r#"SELECT to_jsonb(entry)::TEXT FROM "LiteLLM_ProxyModelTable" entry
    WHERE entry.model_name = $1 AND entry.model_id <> ALL($2::text[]) LIMIT $3"#;
const FIND_BY_CONTAINS_SQL: &str = r#"SELECT to_jsonb(entry)::TEXT FROM "LiteLLM_ProxyModelTable" entry
    WHERE entry.model_name ILIKE $1 AND entry.model_id <> ALL($2::text[]) LIMIT $3"#;

#[derive(Clone, Copy)]
pub struct PostgresSearchDatabase<'db> {
    pub client: Option<PrismaClient<'db>>,
}

#[async_trait]
impl SearchDatabase for PostgresSearchDatabase<'_> {
    async fn count(&self, filter: &ModelFilter) -> Result<i64, SearchDatabaseError> {
        count_rows(self.reader()?, filter).await
    }

    async fn find(
        &self,
        filter: &ModelFilter,
        take: i64,
    ) -> Result<Vec<ProxyModelTable>, SearchDatabaseError> {
        find_rows(self.reader()?, filter, take).await
    }
}

impl PostgresSearchDatabase<'_> {
    fn reader(&self) -> Result<&PostgresStore, SearchDatabaseError> {
        let Some(client) = self.client else {
            let missing = DbError::<Infallible>::NoDbConnected;
            return Err(SearchDatabaseError::Database(TableError::from(missing)));
        };
        if client
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
        {
            return Ok(client.writer);
        }
        Ok(client.reader)
    }
}

fn name_value(filter: &ModelFilter) -> String {
    match &filter.exact_name {
        Some(name) => name.clone(),
        None => format!("%{}%", filter.search_lower),
    }
}

fn excluded_ids(filter: &ModelFilter) -> Vec<String> {
    filter.excluded_ids.iter().cloned().collect()
}

fn database_error(error: DbError) -> SearchDatabaseError {
    SearchDatabaseError::Database(TableError::from(error))
}

fn query_error(error: tokio_postgres::Error) -> SearchDatabaseError {
    database_error(backend::<Infallible>(error))
}

async fn count_rows(
    store: &PostgresStore,
    filter: &ModelFilter,
) -> Result<i64, SearchDatabaseError> {
    let client = store.session().await.map_err(database_error)?;
    let value = name_value(filter);
    let excluded = excluded_ids(filter);
    let parameters: [&(dyn ToSql + Sync); 2] = [&value, &excluded];
    let sql = if filter.exact_name.is_some() {
        COUNT_BY_NAME_SQL
    } else {
        COUNT_BY_CONTAINS_SQL
    };
    let row = client
        .query_one(sql, &parameters)
        .await
        .map_err(query_error)?;
    row.try_get(0).map_err(query_error)
}

async fn find_rows(
    store: &PostgresStore,
    filter: &ModelFilter,
    take: i64,
) -> Result<Vec<ProxyModelTable>, SearchDatabaseError> {
    let client = store.session().await.map_err(database_error)?;
    let value = name_value(filter);
    let excluded = excluded_ids(filter);
    let parameters: [&(dyn ToSql + Sync); 3] = [&value, &excluded, &take];
    let sql = if filter.exact_name.is_some() {
        FIND_BY_NAME_SQL
    } else {
        FIND_BY_CONTAINS_SQL
    };
    let rows = client.query(sql, &parameters).await.map_err(query_error)?;
    let mut found = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row.try_get(0).map_err(query_error)?;
        let stored: JsonData = serde_json::from_str(&text).map_err(SearchDatabaseError::Row)?;
        found.push(ProxyModelTable::try_from(stored).map_err(SearchDatabaseError::Decode)?);
    }
    Ok(found)
}

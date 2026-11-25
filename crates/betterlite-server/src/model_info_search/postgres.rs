use std::convert::Infallible;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_rows::model::ProxyModelTable;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::search::{ModelFilter, SearchDatabase, SearchDatabaseError};

// Prisma's contains reaches Postgres unescaped, so % _ and \ stay LIKE metacharacters and equals is a plain equality.
const COUNT_BY_NAME_SQL: &str = r#"SELECT COUNT(*)::BIGINT FROM "LiteLLM_ProxyModelTable" entry
    WHERE entry.model_name = $1 AND entry.model_id <> ALL($2::text[]) LIMIT $3"#;
const COUNT_BY_CONTAINS_SQL: &str = r#"SELECT COUNT(*)::BIGINT FROM "LiteLLM_ProxyModelTable" entry
    WHERE entry.model_name ILIKE $1 AND entry.model_id <> ALL($2::text[]) LIMIT $3"#;
// A count binds the fetch's $3 as a NULL LIMIT, which Postgres reads as no limit.
const COUNT_SQLS: (&str, &str) = (COUNT_BY_NAME_SQL, COUNT_BY_CONTAINS_SQL);
const FIND_BY_NAME_SQL: &str = r#"SELECT to_jsonb(entry)::TEXT FROM "LiteLLM_ProxyModelTable" entry
    WHERE entry.model_name = $1 AND entry.model_id <> ALL($2::text[]) LIMIT $3"#;
const FIND_BY_CONTAINS_SQL: &str = r#"SELECT to_jsonb(entry)::TEXT FROM "LiteLLM_ProxyModelTable" entry
    WHERE entry.model_name ILIKE $1 AND entry.model_id <> ALL($2::text[]) LIMIT $3"#;
const FIND_SQLS: (&str, &str) = (FIND_BY_NAME_SQL, FIND_BY_CONTAINS_SQL);

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

impl From<tokio_postgres::Error> for SearchDatabaseError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(TableError::from(backend::<Infallible>(error)))
    }
}

async fn search_rows(
    store: &PostgresStore,
    filter: &ModelFilter,
    sqls: (&'static str, &'static str),
    take: Option<i64>,
) -> Result<Vec<Row>, SearchDatabaseError> {
    let value = name_value(filter);
    let excluded: Vec<String> = filter.excluded_ids.iter().cloned().collect();
    let sql = if filter.exact_name.is_some() {
        sqls.0
    } else {
        sqls.1
    };
    let parameters: [&(dyn ToSql + Sync); 3] = [&value, &excluded, &take];
    let client = store
        .session()
        .await
        .map_err(|error| SearchDatabaseError::Database(TableError::from(error)))?;
    client
        .query(sql, &parameters)
        .await
        .map_err(SearchDatabaseError::from)
}

async fn count_rows(
    store: &PostgresStore,
    filter: &ModelFilter,
) -> Result<i64, SearchDatabaseError> {
    let rows = search_rows(store, filter, COUNT_SQLS, None).await?;
    let Some(row) = rows.first() else {
        return Err(SearchDatabaseError::MissingRow);
    };
    row.try_get(0)?
}

async fn find_rows(
    store: &PostgresStore,
    filter: &ModelFilter,
    take: i64,
) -> Result<Vec<ProxyModelTable>, SearchDatabaseError> {
    let rows = search_rows(store, filter, FIND_SQLS, Some(take)).await?;
    let mut found = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row.try_get(0)?;
        let stored: JsonData = serde_json::from_str(&text).map_err(SearchDatabaseError::Row)?;
        found.push(ProxyModelTable::try_from(stored).map_err(SearchDatabaseError::Decode)?);
    }
    Ok(found)
}

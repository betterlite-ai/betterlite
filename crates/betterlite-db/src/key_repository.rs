pub mod decode;
pub mod record;
pub mod table;

mod tests;

use crate::database_replica::client::DatabaseClient;
use crate::database_replica::pinned::WriterPinnedClient;
use crate::database_replica::routing::{Route, RouteError};
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::key_hash::KeyHash;
use crate::postgres_store::diagnostics::PoolFailure;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::repository::Repository;
use crate::postgres_store::rows::KeyColumn;
use deadpool_postgres::Pool;
use decode::TokenDecodeError;
use std::error::Error;
use table::columns::TokenColumn;
use table::converted::ConvertedTokenTable;
use table::{
    BoundQuery, RawTokenRow, TOKEN_TABLE, TOKEN_TABLE_ATTRIBUTE, TokenFilter, TokenParameter,
    TokenTableError, TokenValue, TokenWrite,
};
use tokio_postgres::types::ToSql;

pub type TokenModel = LiteLLMVerificationToken;

#[derive(Debug, thiserror::Error)]
pub enum TokenRepositoryError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Table(#[from] TokenTableError),
    #[error(transparent)]
    Route(#[from] RouteError<ClientError>),
}

pub trait TokenTableAccess: Send + Sync {
    fn pool(&self) -> &Pool;
}

impl TokenTableAccess for Pool {
    fn pool(&self) -> &Pool {
        self
    }
}

pub struct VerificationTokenRepository<'db, C: DatabaseClient> {
    pub database: Option<WriterPinnedClient<'db, C>>,
}

impl<C: DatabaseClient> VerificationTokenRepository<'_, C> {
    pub fn table(&self) -> Result<TokenTable<'_, C::Attribute>, TokenRepositoryError<C::Error>>
    where
        C::Attribute: TokenTableAccess,
    {
        let database = self
            .database
            .as_ref()
            .ok_or(TokenTableError::Database(DbError::NoDbConnected))?;
        Ok(TokenTable {
            route: database.attribute(TOKEN_TABLE_ATTRIBUTE)?,
        })
    }

    pub async fn find_many_in(
        &self,
        keys: Vec<KeyHash>,
    ) -> Result<Vec<TokenModel>, TokenRepositoryError<C::Error>>
    where
        C::Attribute: TokenTableAccess,
    {
        let client = self.table()?.connection("find_many").await?;
        Repository {
            client: Some(&client),
            table: ConvertedTokenTable,
        }
        .find_many_in(KeyColumn::Id, keys)
        .await
        .map_err(TokenTableError::Database)
        .map_err(TokenRepositoryError::Table)
    }
}

pub struct TokenTable<'route, Accessor> {
    route: Route<'route, Accessor>,
}

impl<Accessor> TokenTable<'_, Accessor>
where
    Accessor: TokenTableAccess,
{
    async fn connection(&self, method: &str) -> Result<deadpool_postgres::Client, TokenTableError> {
        let pool = self.route.select(method).pool();
        pool.get().await.map_err(|driver| {
            TokenTableError::Database(DbError::Pool {
                source: PoolFailure { driver },
            })
        })
    }

    async fn statement(
        &self,
        method: &str,
        sql: &str,
        query: &BoundQuery,
    ) -> Result<Vec<RawTokenRow>, TokenTableError> {
        let client = self.connection(method).await?;
        let parameters: Vec<&(dyn ToSql + Sync)> =
            query.parameters.iter().map(TokenParameter::bind).collect();
        let rows = client
            .query(sql, &parameters)
            .await
            .map_err(backend::<TokenDecodeError>)?;
        rows.iter()
            .map(RawTokenRow::from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(TokenTableError::Decode)
    }

    pub async fn find_unique(&self, key: &KeyHash) -> Result<Option<RawTokenRow>, TokenTableError> {
        let filter = TokenFilter(vec![(TokenColumn::Token, TokenValue::KeyHash(key.clone()))]);
        let mut query = BoundQuery::default();
        query.filter(&filter, false)?;
        let sql = query.select_sql(None);
        let rows = self.statement("find_unique", &sql, &query).await?;
        Ok(rows.into_iter().next())
    }

    pub async fn find_many(
        &self,
        filter: &TokenFilter,
        take: Option<i64>,
    ) -> Result<Vec<RawTokenRow>, TokenTableError> {
        let mut query = BoundQuery::default();
        query.filter(filter, false)?;
        let limit = take.map(|value| query.limit(value));
        let sql = query.select_sql(limit.as_deref());
        self.statement("find_many", &sql, &query).await
    }

    pub async fn create(&self, data: &TokenWrite) -> Result<RawTokenRow, TokenTableError> {
        let mut query = BoundQuery::default();
        let (columns, placeholders) = query.insert_columns(data)?;
        let sql = BoundQuery::insert_sql(&columns, &placeholders, None);
        let rows = self.statement("create", &sql, &query).await?;
        rows.into_iter()
            .next()
            .ok_or(TokenTableError::Database(DbError::CreateNull))
    }

    pub async fn update(
        &self,
        filter: &TokenFilter,
        data: &TokenWrite,
    ) -> Result<Vec<RawTokenRow>, TokenTableError> {
        let mut query = BoundQuery::default();
        let assignments = query.assignments(data)?;
        query.filter(filter, true)?;
        let clause = table::assignment_clause(&assignments, "\"token\" = \"token\"");
        let sql = query.statement_sql(format!("UPDATE {TOKEN_TABLE} AS r SET {clause}"));
        self.statement("update", &sql, &query).await
    }

    pub async fn delete(&self, filter: &TokenFilter) -> Result<Vec<RawTokenRow>, TokenTableError> {
        let mut query = BoundQuery::default();
        query.filter(filter, true)?;
        let sql = query.statement_sql(format!("DELETE FROM {TOKEN_TABLE} AS r"));
        self.statement("delete", &sql, &query).await
    }

    pub async fn upsert(
        &self,
        create: &TokenWrite,
        update: &TokenWrite,
    ) -> Result<RawTokenRow, TokenTableError> {
        let mut query = BoundQuery::default();
        let (columns, placeholders) = query.insert_columns(create)?;
        let assignments = query.assignments(update)?;
        let clause = table::assignment_clause(&assignments, "\"token\" = EXCLUDED.\"token\"");
        let sql = BoundQuery::insert_sql(&columns, &placeholders, Some(&clause));
        let rows = self.statement("upsert", &sql, &query).await?;
        rows.into_iter()
            .next()
            .ok_or(TokenTableError::Database(DbError::CreateNull))
    }
}

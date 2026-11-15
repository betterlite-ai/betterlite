pub mod find;

use super::rows::records::{
    DeletedVerificationTokenRow, VerificationTokenRow, deleted_verification_token_row,
    verification_token_row,
};
use super::rows::{DatabaseRow, RowError};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityRow, SqlParameter, SqlQuery};
use crate::postgres_store::errors::{DbError, backend};
use deadpool_postgres::Client;
use std::future::Future;
use std::pin::Pin;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

const ACTIVE_TOKENS_SQL: &str = "SELECT token, key_alias, team_id, user_id, metadata::text AS metadata FROM \"LiteLLM_VerificationToken\" WHERE token = ANY($1)";
const DELETED_TOKENS_SQL: &str = "SELECT token, key_alias, team_id, user_id, metadata::text AS metadata, deleted_at FROM \"LiteLLM_DeletedVerificationToken\" WHERE token = ANY($1)";

pub type DatabaseFuture<'database, T> =
    Pin<Box<dyn Future<Output = Result<T, DbError<RowError>>> + Send + 'database>>;

type CountFuture<'database> = DatabaseFuture<'database, i64>;
type TokenFuture<'database, T> = DatabaseFuture<'database, Vec<T>>;

pub trait DailyActivityDatabase: Send + Sync {
    fn query_raw<'database>(
        &'database self,
        query: &'database SqlQuery,
    ) -> DatabaseFuture<'database, Option<Vec<DatabaseRow>>>;

    fn active_tokens<'database>(
        &'database self,
        tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<VerificationTokenRow>>;

    fn deleted_tokens<'database>(
        &'database self,
        tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<DeletedVerificationTokenRow>>;

    fn count_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        predicate: &'database SqlQuery,
    ) -> DatabaseFuture<'database, i64>;

    fn find_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        predicate: &'database SqlQuery,
        skip: i64,
        take: i64,
    ) -> DatabaseFuture<'database, Vec<DailyActivityRow>>;
}

pub struct PostgresDailyActivityDatabase<'client> {
    pub writer: &'client Client,
    pub reader: Option<&'client Client>,
}

impl DailyActivityDatabase for PostgresDailyActivityDatabase<'_> {
    fn query_raw<'database>(
        &'database self,
        query: &'database SqlQuery,
    ) -> DatabaseFuture<'database, Option<Vec<DatabaseRow>>> {
        Box::pin(async move {
            let params = bind_params(&query.params);
            let rows = self
                .writer
                .query(query.sql.as_str(), &params)
                .await
                .map_err(backend)?;
            let decoded = rows
                .iter()
                .map(|row| {
                    DatabaseRow::decode(row).map_err(|source| DbError::InvalidRecord { source })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(decoded))
        })
    }

    fn active_tokens<'database>(
        &'database self,
        tokens: &'database [String],
    ) -> TokenFuture<'database, VerificationTokenRow> {
        Box::pin(async move { decode_tokens(&token_rows(self, tokens, ACTIVE_TOKENS_SQL).await?) })
    }

    fn deleted_tokens<'database>(
        &'database self,
        tokens: &'database [String],
    ) -> TokenFuture<'database, DeletedVerificationTokenRow> {
        Box::pin(async move {
            decode_deleted_tokens(&token_rows(self, tokens, DELETED_TOKENS_SQL).await?)
        })
    }

    fn count_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        predicate: &'database SqlQuery,
    ) -> CountFuture<'database> {
        Box::pin(count_daily_query(
            self.reader.unwrap_or(self.writer),
            table,
            predicate,
        ))
    }

    fn find_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        predicate: &'database SqlQuery,
        skip: i64,
        take: i64,
    ) -> DatabaseFuture<'database, Vec<DailyActivityRow>> {
        Box::pin(find::find_daily_query(
            self.reader.unwrap_or(self.writer),
            table,
            predicate,
            skip,
            take,
        ))
    }
}

async fn count_daily_query(
    client: &Client,
    table: DailyActivityTable,
    predicate: &SqlQuery,
) -> Result<i64, DbError<RowError>> {
    let relation = table.metadata().relation;
    let sql = format!(
        "SELECT COUNT(*)::bigint FROM \"{relation}\" WHERE {}",
        predicate.sql
    );
    let params = bind_params(&predicate.params);
    let row = client
        .query_one(sql.as_str(), &params)
        .await
        .map_err(backend)?;
    row.try_get::<_, i64>(0).map_err(backend)
}

fn token_rows<'database>(
    database: &'database PostgresDailyActivityDatabase<'_>,
    tokens: &'database [String],
    sql: &'static str,
) -> DatabaseFuture<'database, Vec<Row>> {
    Box::pin(async move {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let values = tokens.to_vec();
        let params: [&(dyn ToSql + Sync); 1] = [&values];
        let client = database.reader.unwrap_or(database.writer);
        client.query(sql, &params).await.map_err(backend)
    })
}

fn decode_tokens(rows: &[Row]) -> Result<Vec<VerificationTokenRow>, DbError<RowError>> {
    rows.iter()
        .map(|row| verification_token_row(row).map_err(|source| DbError::InvalidRecord { source }))
        .collect()
}

fn decode_deleted_tokens(
    rows: &[Row],
) -> Result<Vec<DeletedVerificationTokenRow>, DbError<RowError>> {
    rows.iter()
        .map(|row| {
            deleted_verification_token_row(row).map_err(|source| DbError::InvalidRecord { source })
        })
        .collect()
}

fn bind_params(parameters: &[SqlParameter]) -> Vec<&(dyn ToSql + Sync)> {
    let mut binds: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        match parameter {
            SqlParameter::Text(value) => binds.push(value),
            SqlParameter::Texts(values) => binds.push(values),
            SqlParameter::Integer(value) => binds.push(value),
        }
    }
    binds
}

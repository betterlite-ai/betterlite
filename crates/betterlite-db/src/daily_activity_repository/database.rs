use super::rows::records::{
    DeletedVerificationTokenRow, VerificationTokenRow, daily_activity_row,
    deleted_verification_token_row, verification_token_row,
};
use super::rows::{DatabaseRow, RowError};
use crate::daily_activity_contracts::{
    DailyActivityRow, DailyActivityTable, SqlParameter, SqlQuery,
};
use crate::postgres_store::errors::{DbError, backend};
use deadpool_postgres::Client;
use std::future::Future;
use std::pin::Pin;
use tokio_postgres::types::ToSql;

const ACTIVE_TOKENS_SQL: &str = "SELECT token, key_alias, team_id, user_id, metadata::text AS metadata FROM \"LiteLLM_VerificationToken\" WHERE token = ANY($1)";
const DELETED_TOKENS_SQL: &str = "SELECT token, key_alias, team_id, user_id, metadata::text AS metadata, deleted_at FROM \"LiteLLM_DeletedVerificationToken\" WHERE token = ANY($1)";

pub type DatabaseFuture<'database, T> =
    Pin<Box<dyn Future<Output = Result<T, DbError<RowError>>> + Send + 'database>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DailyRowFilter {
    DateRange { start: String, end: String },
    EntityIn { values: Vec<String> },
    EntityNotIn { values: Vec<String> },
    Model { value: String },
    ApiKeyIn { values: Vec<String> },
}

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
        filters: &'database [DailyRowFilter],
    ) -> DatabaseFuture<'database, i64>;

    fn find_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        filters: &'database [DailyRowFilter],
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
            let mut decoded = Vec::with_capacity(rows.len());
            for row in &rows {
                decoded.push(
                    DatabaseRow::decode(row).map_err(|source| DbError::InvalidRecord { source })?,
                );
            }
            Ok(Some(decoded))
        })
    }

    fn active_tokens<'database>(
        &'database self,
        tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<VerificationTokenRow>> {
        Box::pin(async move {
            if tokens.is_empty() {
                return Ok(Vec::new());
            }
            let values = tokens.to_vec();
            let params: [&(dyn ToSql + Sync); 1] = [&values];
            let client = self.reader.unwrap_or(self.writer);
            let rows = client
                .query(ACTIVE_TOKENS_SQL, &params)
                .await
                .map_err(backend)?;
            let mut decoded = Vec::with_capacity(rows.len());
            for row in &rows {
                decoded.push(
                    verification_token_row(row)
                        .map_err(|source| DbError::InvalidRecord { source })?,
                );
            }
            Ok(decoded)
        })
    }

    fn deleted_tokens<'database>(
        &'database self,
        tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<DeletedVerificationTokenRow>> {
        Box::pin(async move {
            if tokens.is_empty() {
                return Ok(Vec::new());
            }
            let values = tokens.to_vec();
            let params: [&(dyn ToSql + Sync); 1] = [&values];
            let client = self.reader.unwrap_or(self.writer);
            let rows = client
                .query(DELETED_TOKENS_SQL, &params)
                .await
                .map_err(backend)?;
            let mut decoded = Vec::with_capacity(rows.len());
            for row in &rows {
                decoded.push(
                    deleted_verification_token_row(row)
                        .map_err(|source| DbError::InvalidRecord { source })?,
                );
            }
            Ok(decoded)
        })
    }

    fn count_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        filters: &'database [DailyRowFilter],
    ) -> DatabaseFuture<'database, i64> {
        Box::pin(async move {
            let (where_sql, parameters) = render_filters(table, filters);
            let sql = format!(
                "SELECT COUNT(*)::bigint FROM \"{}\" WHERE {where_sql}",
                table.metadata().relation
            );
            let params = bind_params(&parameters);
            let client = self.reader.unwrap_or(self.writer);
            let row = client
                .query_one(sql.as_str(), &params)
                .await
                .map_err(backend)?;
            row.try_get::<_, i64>(0).map_err(backend)
        })
    }

    fn find_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        filters: &'database [DailyRowFilter],
        skip: i64,
        take: i64,
    ) -> DatabaseFuture<'database, Vec<DailyActivityRow>> {
        Box::pin(async move {
            let (where_sql, mut parameters) = render_filters(table, filters);
            let limit_index = parameters.len().saturating_add(1);
            let offset_index = parameters.len().saturating_add(2);
            parameters.push(SqlParameter::Integer(take));
            parameters.push(SqlParameter::Integer(skip));
            let sql = format!(
                "SELECT * FROM \"{}\" WHERE {where_sql} ORDER BY date DESC, id ASC LIMIT ${limit_index} OFFSET ${offset_index}",
                table.metadata().relation
            );
            let params = bind_params(&parameters);
            let client = self.reader.unwrap_or(self.writer);
            let rows = client.query(sql.as_str(), &params).await.map_err(backend)?;
            let mut decoded = Vec::with_capacity(rows.len());
            for row in &rows {
                decoded.push(
                    daily_activity_row(row).map_err(|source| DbError::InvalidRecord { source })?,
                );
            }
            Ok(decoded)
        })
    }
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

fn render_filters(
    table: DailyActivityTable,
    filters: &[DailyRowFilter],
) -> (String, Vec<SqlParameter>) {
    let entity = format!("\"{}\"", table.metadata().entity_field);
    let mut clauses: Vec<String> = Vec::with_capacity(filters.len());
    let mut parameters: Vec<SqlParameter> = Vec::new();
    for filter in filters {
        match filter {
            DailyRowFilter::DateRange { start, end } => {
                let start_index = parameters.len().saturating_add(1);
                let end_index = parameters.len().saturating_add(2);
                clauses.push(format!("date >= ${start_index} AND date <= ${end_index}"));
                parameters.push(SqlParameter::Text(start.clone()));
                parameters.push(SqlParameter::Text(end.clone()));
            }
            DailyRowFilter::EntityIn { values } => {
                if values.is_empty() {
                    clauses.push("FALSE".to_owned());
                } else {
                    let index = parameters.len().saturating_add(1);
                    clauses.push(format!("{entity} = ANY(${index}::text[])"));
                    parameters.push(SqlParameter::Texts(values.clone()));
                }
            }
            DailyRowFilter::EntityNotIn { values } => {
                if values.is_empty() {
                    continue;
                }
                let index = parameters.len().saturating_add(1);
                clauses.push(format!(
                    "({entity} IS NULL OR NOT ({entity} = ANY(${index}::text[])))"
                ));
                parameters.push(SqlParameter::Texts(values.clone()));
            }
            DailyRowFilter::Model { value } => {
                let index = parameters.len().saturating_add(1);
                clauses.push(format!("model = ${index}"));
                parameters.push(SqlParameter::Text(value.clone()));
            }
            DailyRowFilter::ApiKeyIn { values } => {
                if values.is_empty() {
                    clauses.push("FALSE".to_owned());
                } else {
                    let index = parameters.len().saturating_add(1);
                    clauses.push(format!("api_key = ANY(${index}::text[])"));
                    parameters.push(SqlParameter::Texts(values.clone()));
                }
            }
        }
    }
    (clauses.join(" AND "), parameters)
}

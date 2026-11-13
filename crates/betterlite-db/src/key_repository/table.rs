pub mod columns;

pub(super) mod converted;

use self::columns::{TokenColumn, TokenSqlType};
use crate::budget_permission_rows::{JsonData, Timestamp};
use crate::key_hash::KeyHash;
use crate::key_repository::decode::TokenDecodeError;
use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::errors::DbError;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::fmt;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

pub const TOKEN_TABLE_ATTRIBUTE: &str = "litellm_verificationtoken";

pub(super) const TOKEN_TABLE: &str = "\"LiteLLM_VerificationToken\"";
const TOKEN_RECORD: &str = "row_to_json(r)::text";
const SELECT_TOKEN_SQL: &str =
    "SELECT row_to_json(r)::text FROM \"LiteLLM_VerificationToken\" r WHERE r.\"token\" = $1";
const DELETE_TOKEN_SQL: &str = "DELETE FROM \"LiteLLM_VerificationToken\" AS r WHERE r.\"token\" = $1 RETURNING row_to_json(r)::text";
const MEMBERSHIP_TOKEN_SQL: &str =
    "SELECT row_to_json(r)::text FROM \"LiteLLM_VerificationToken\" r WHERE r.\"token\" = ANY($1)";
const COUNT_TOKEN_SQL: &str = "SELECT count(*) FROM \"LiteLLM_VerificationToken\"";
const LIST_TOKEN_SQL: &str = "SELECT row_to_json(r)::text FROM \"LiteLLM_VerificationToken\" r";
const TOUCH_TOKEN_SQL: &str = "UPDATE \"LiteLLM_VerificationToken\" AS r SET \"token\" = \"token\" WHERE r.\"token\" = $1 RETURNING row_to_json(r)::text";
const DEFAULT_INSERT_SQL: &str =
    "INSERT INTO \"LiteLLM_VerificationToken\" AS r DEFAULT VALUES RETURNING row_to_json(r)::text";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenValue {
    Text(String),
    KeyHash(KeyHash),
    Bool(bool),
    Integer(i64),
    Number(Decimal),
    Json(JsonData),
    Timestamp(Timestamp),
    TextArray(Vec<String>),
    Null,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenFilter(pub Vec<(TokenColumn, TokenValue)>);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenWrite(pub BTreeMap<TokenColumn, TokenValue>);

#[derive(Clone, PartialEq, Eq)]
pub struct RawTokenRow(pub JsonData);

impl fmt::Debug for RawTokenRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RawTokenRow([REDACTED])")
    }
}

impl RawTokenRow {
    pub fn from_row(row: &Row) -> Result<Self, TokenDecodeError> {
        let text: String = row.try_get(0).map_err(|driver| TokenDecodeError::Column {
            source: DriverError { driver },
        })?;
        let value: JsonData = serde_json::from_str(&text).map_err(TokenDecodeError::RowJson)?;
        Ok(Self(value))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenTableError {
    #[error(transparent)]
    Database(#[from] DbError<TokenDecodeError>),
    #[error(transparent)]
    Decode(#[from] TokenDecodeError),
    #[error("token write value could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("a token write needs at least one filter column")]
    MissingFilter,
}

pub(super) enum TokenParameter {
    Text(Option<String>),
    Array(Option<Vec<String>>),
    Integer(i64),
}

impl TokenParameter {
    pub(super) fn bind(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(value) => value,
            Self::Array(value) => value,
            Self::Integer(value) => value,
        }
    }

    fn placeholder(&self, index: usize, column_type: TokenSqlType) -> String {
        match self {
            Self::Text(_) if column_type == TokenSqlType::Text => format!("${index}::text"),
            Self::Text(_) => format!("${index}::text::{}", column_type.cast()),
            Self::Array(_) => format!("${index}::text[]"),
            Self::Integer(_) => format!("${index}::bigint"),
        }
    }
}

#[derive(Default)]
pub(super) struct BoundQuery {
    pub(super) parameters: Vec<TokenParameter>,
    conditions: Vec<String>,
}

impl BoundQuery {
    pub(super) fn bind(
        &mut self,
        column: TokenColumn,
        value: &TokenValue,
    ) -> Result<String, TokenTableError> {
        let index = self.parameters.len().saturating_add(1);
        let parameter = bind_value(column, value)?;
        let placeholder = parameter.placeholder(index, column.sql_type());
        self.parameters.push(parameter);
        Ok(placeholder)
    }

    pub(super) fn filter(
        &mut self,
        filter: &TokenFilter,
        required: bool,
    ) -> Result<(), TokenTableError> {
        if required && filter.0.is_empty() {
            return Err(TokenTableError::MissingFilter);
        }
        for (column, value) in &filter.0 {
            let placeholder = self.bind(*column, value)?;
            self.conditions
                .push(format!("r.\"{}\" = {placeholder}", column.name()));
        }
        Ok(())
    }

    pub(super) fn limit(&mut self, take: i64) -> String {
        let index = self.parameters.len().saturating_add(1);
        self.parameters.push(TokenParameter::Integer(take));
        format!("${index}::bigint")
    }

    pub(super) fn insert_columns(
        &mut self,
        data: &TokenWrite,
    ) -> Result<(Vec<String>, Vec<String>), TokenTableError> {
        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        for (column, value) in &data.0 {
            columns.push(format!("\"{}\"", column.name()));
            placeholders.push(self.bind(*column, value)?);
        }
        Ok((columns, placeholders))
    }

    pub(super) fn assignments(
        &mut self,
        data: &TokenWrite,
    ) -> Result<Vec<String>, TokenTableError> {
        let (columns, placeholders) = self.insert_columns(data)?;
        let assignments = columns
            .into_iter()
            .zip(placeholders)
            .map(|(column, placeholder)| format!("{column} = {placeholder}"))
            .collect();
        Ok(assignments)
    }

    pub(super) fn select_sql(&self, bound_limit: Option<&str>) -> String {
        let mut sql = String::from(LIST_TOKEN_SQL);
        if !self.conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.conditions.join(" AND "));
        }
        if let Some(limit_sql) = bound_limit {
            sql.push_str(" LIMIT ");
            sql.push_str(limit_sql);
        }
        sql
    }

    pub(super) fn statement_sql(&self, head: String) -> String {
        let mut sql = head;
        if !self.conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.conditions.join(" AND "));
        }
        sql.push_str(" RETURNING ");
        sql.push_str(TOKEN_RECORD);
        sql
    }

    pub(super) fn insert_sql(
        columns: &[String],
        placeholders: &[String],
        conflict_clause: Option<&str>,
    ) -> String {
        let mut sql = format!("INSERT INTO {TOKEN_TABLE} AS r");
        if columns.is_empty() {
            sql.push_str(" DEFAULT VALUES");
        } else {
            sql.push_str(" (");
            sql.push_str(&columns.join(", "));
            sql.push_str(") VALUES (");
            sql.push_str(&placeholders.join(", "));
            sql.push(')');
        }
        if let Some(assignment_sql) = conflict_clause {
            sql.push_str(" ON CONFLICT (\"token\") DO UPDATE SET ");
            sql.push_str(assignment_sql);
        }
        sql.push_str(" RETURNING ");
        sql.push_str(TOKEN_RECORD);
        sql
    }
}

pub(super) fn assignment_clause(assignments: &[String], fallback: &str) -> String {
    if assignments.is_empty() {
        String::from(fallback)
    } else {
        assignments.join(", ")
    }
}

fn bind_value(column: TokenColumn, value: &TokenValue) -> Result<TokenParameter, TokenTableError> {
    match value {
        TokenValue::Text(text) => Ok(TokenParameter::Text(Some(text.clone()))),
        TokenValue::KeyHash(key) => Ok(TokenParameter::Text(Some(String::from(
            key.expose_secret(),
        )))),
        TokenValue::Bool(flag) => Ok(TokenParameter::Text(Some(String::from(if *flag {
            "true"
        } else {
            "false"
        })))),
        TokenValue::Integer(number) => Ok(TokenParameter::Text(Some(number.to_string()))),
        TokenValue::Number(number) => Ok(TokenParameter::Text(Some(number.to_string()))),
        TokenValue::Json(json) => Ok(TokenParameter::Text(Some(
            serde_json::to_string(json).map_err(|source| TokenTableError::Encode { source })?,
        ))),
        TokenValue::Timestamp(timestamp) => Ok(TokenParameter::Text(Some(
            timestamp
                .utc()
                .naive_utc()
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string(),
        ))),
        TokenValue::TextArray(items) => Ok(TokenParameter::Array(Some(items.clone()))),
        TokenValue::Null => Ok(match column.sql_type() {
            TokenSqlType::TextArray => TokenParameter::Array(None),
            TokenSqlType::Text
            | TokenSqlType::Boolean
            | TokenSqlType::BigInt
            | TokenSqlType::DoublePrecision
            | TokenSqlType::Json
            | TokenSqlType::Timestamp => TokenParameter::Text(None),
        }),
    }
}

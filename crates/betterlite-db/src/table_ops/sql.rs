use std::collections::BTreeMap;
use std::error::Error;

use secrecy::ExposeSecret as _;
use tokio_postgres::types::ToSql;

use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenSqlType;

use super::TableOpsError;
use super::columns::TableColumn;

// every payload value is bound as its own type, so Postgres reads the column type instead of a text cast
pub(super) enum Bound {
    Text(Option<String>),
    Bool(Option<bool>),
    Integer(Option<i64>),
    Array(Option<Vec<String>>),
}

impl Bound {
    fn into_sql(self) -> Box<dyn ToSql + Sync> {
        match self {
            Self::Text(value) => Box::new(value),
            Self::Bool(value) => Box::new(value),
            Self::Integer(value) => Box::new(value),
            Self::Array(value) => Box::new(value),
        }
    }
}

pub(super) struct Parameters {
    values: Vec<Bound>,
}

impl Parameters {
    pub(super) fn new() -> Self {
        Self { values: Vec::new() }
    }

    fn push(&mut self, bound: Bound) -> usize {
        self.values.push(bound);
        self.values.len()
    }

    pub(super) fn bind_values(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.values
            .iter()
            .map(|value| match value {
                Bound::Text(inner) => inner as &(dyn ToSql + Sync),
                Bound::Bool(inner) => inner as &(dyn ToSql + Sync),
                Bound::Integer(inner) => inner as &(dyn ToSql + Sync),
                Bound::Array(inner) => inner as &(dyn ToSql + Sync),
            })
            .collect()
    }
}

pub(super) struct Statement {
    pub(super) sql: String,
    pub(super) parameters: Parameters,
}

pub(super) fn bind_literal<ClientError>(
    parameters: &mut Parameters,
    column: &'static str,
    value: &TokenValue,
    kind: TokenSqlType,
) -> Result<String, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    let (bound, cast) = match kind {
        TokenSqlType::Text => {
            let value = match value {
                TokenValue::Text(text) => Bound::Text(Some(text.clone())),
                TokenValue::KeyHash(hash) => Bound::Text(Some(String::from(hash.expose_secret()))),
                TokenValue::Null => Bound::Text(None),
                _ => return Err(TableOpsError::WrongType { column }),
            };
            (value, "")
        }
        TokenSqlType::Boolean => {
            let value = match value {
                TokenValue::Bool(flag) => Bound::Bool(Some(*flag)),
                TokenValue::Null => Bound::Bool(None),
                _ => return Err(TableOpsError::WrongType { column }),
            };
            (value, "")
        }
        TokenSqlType::BigInt => {
            let value = match value {
                TokenValue::Integer(number) => Bound::Integer(Some(*number)),
                TokenValue::Null => Bound::Integer(None),
                _ => return Err(TableOpsError::WrongType { column }),
            };
            (value, "")
        }
        TokenSqlType::TextArray => {
            let value = match value {
                TokenValue::TextArray(items) => Bound::Array(Some(items.clone())),
                TokenValue::Null => Bound::Array(None),
                _ => return Err(TableOpsError::WrongType { column }),
            };
            (value, "")
        }
        TokenSqlType::DoublePrecision => (
            Bound::Text(encoded_text(column, value)?),
            "::text::double precision",
        ),
        TokenSqlType::Json => (Bound::Text(encoded_text(column, value)?), "::text::jsonb"),
        TokenSqlType::Timestamp => (
            Bound::Text(encoded_text(column, value)?),
            "::text::timestamp(3)",
        ),
    };
    let index = parameters.push(bound);
    Ok(format!("${index}{cast}"))
}

fn encoded_text<ClientError>(
    column: &'static str,
    value: &TokenValue,
) -> Result<Option<String>, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    match value {
        TokenValue::Number(number) => Ok(Some(number.to_string())),
        TokenValue::Json(json) => serde_json::to_string(json)
            .map(Some)
            .map_err(|source| TableOpsError::Encode { column, source }),
        TokenValue::Timestamp(timestamp) => Ok(Some(
            timestamp
                .utc()
                .naive_utc()
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string(),
        )),
        TokenValue::Text(text) => Ok(Some(text.clone())),
        TokenValue::Null => Ok(None),
        TokenValue::KeyHash(_)
        | TokenValue::Bool(_)
        | TokenValue::Integer(_)
        | TokenValue::TextArray(_) => Err(TableOpsError::WrongType { column }),
    }
}

// Prisma's upsert inserts the create row and, on a key conflict, writes only the update row's values
pub(super) fn upsert<ClientError, Column>(
    create: &BTreeMap<Column, TokenValue>,
    update: Option<&BTreeMap<Column, TokenValue>>,
) -> Result<Statement, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
    Column: TableColumn,
{
    let (key_name, _) = Column::KEY.definition();
    if !create.contains_key(&Column::KEY) {
        return Err(TableOpsError::MissingField { field: key_name });
    }
    let mut parameters = Parameters::new();
    let mut names = Vec::new();
    let mut placeholders = Vec::new();
    for (column, value) in create {
        let (name, kind) = column.definition();
        names.push(format!("\"{name}\""));
        placeholders.push(bind_literal(&mut parameters, name, value, kind)?);
    }
    let mut assignments = Vec::new();
    for (column, value) in update.into_iter().flatten() {
        let (name, kind) = column.definition();
        let placeholder = bind_literal(&mut parameters, name, value, kind)?;
        assignments.push(format!("\"{name}\" = {placeholder}"));
    }
    if assignments.is_empty() {
        assignments.push(format!("\"{key_name}\" = EXCLUDED.\"{key_name}\""));
    }
    let sql = format!(
        "INSERT INTO \"{}\" AS stored ({}) VALUES ({}) ON CONFLICT (\"{key_name}\") DO UPDATE SET {} RETURNING row_to_json(stored)::text",
        Column::TABLE,
        names.join(", "),
        placeholders.join(", "),
        assignments.join(", "),
    );
    Ok(Statement { sql, parameters })
}

pub(super) fn update_by_key<ClientError, Column>(
    payload: &BTreeMap<Column, TokenValue>,
) -> Result<Statement, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
    Column: TableColumn,
{
    let (key_name, _) = Column::KEY.definition();
    let key = match payload.get(&Column::KEY) {
        Some(TokenValue::Text(text)) => text.clone(),
        Some(TokenValue::KeyHash(hash)) => String::from(hash.expose_secret()),
        Some(_) => return Err(TableOpsError::WrongType { column: key_name }),
        None => return Err(TableOpsError::MissingField { field: key_name }),
    };
    let mut parameters = Parameters::new();
    let mut assignments = Vec::new();
    for (column, value) in payload {
        if *column == Column::KEY {
            continue;
        }
        let (name, kind) = column.definition();
        let placeholder = bind_literal(&mut parameters, name, value, kind)?;
        assignments.push(format!("\"{name}\" = {placeholder}"));
    }
    let key_index = parameters.push(Bound::Text(Some(key)));
    if assignments.is_empty() {
        assignments.push(format!("\"{key_name}\" = ${key_index}"));
    }
    let sql = format!(
        "UPDATE \"{}\" AS stored SET {} WHERE stored.\"{key_name}\" = ${key_index}",
        Column::TABLE,
        assignments.join(", "),
    );
    Ok(Statement { sql, parameters })
}

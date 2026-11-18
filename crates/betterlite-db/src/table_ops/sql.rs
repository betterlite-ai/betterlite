use std::collections::BTreeMap;
use std::error::Error;

use secrecy::ExposeSecret as _;
use tokio_postgres::types::ToSql;

use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenSqlType;

use super::TableOpsError;
use super::columns::TableColumn;

// every payload value is bound with the Rust type its column needs, so Postgres never reads a payload as text
pub(super) enum Bound {
    Text(Option<String>),
    Bool(Option<bool>),
    Integer(Option<i64>),
    Array(Option<Vec<String>>),
}

impl Bound {
    fn as_sql(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(value) => value,
            Self::Bool(value) => value,
            Self::Integer(value) => value,
            Self::Array(value) => value,
        }
    }
}

#[derive(Default)]
pub(super) struct Parameters {
    values: Vec<Bound>,
}

impl Parameters {
    fn push(&mut self, bound: Bound) -> usize {
        self.values.push(bound);
        self.values.len()
    }

    pub(super) fn bind_values(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.values.iter().map(Bound::as_sql).collect()
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
            // Postgres assigns a bigint parameter to the int4 columns Prisma declares, and rejects an overflow
            (value, "::bigint")
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

fn bind_payload<ClientError, Column>(
    payload: &BTreeMap<Column, TokenValue>,
    parameters: &mut Parameters,
) -> Result<Vec<(&'static str, String)>, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
    Column: TableColumn,
{
    let mut bound = Vec::with_capacity(payload.len());
    for (column, value) in payload {
        let (name, kind) = column.definition();
        let placeholder = bind_literal(parameters, name, value, kind)?;
        bound.push((name, placeholder));
    }
    Ok(bound)
}

// what Prisma does when the inserted row's key already exists
#[derive(Clone, Copy)]
pub(super) enum InsertConflict<'payload, Column> {
    // create raises, as the database's own unique key does
    Reject,
    // upsert keeps the stored row and returns it
    Keep,
    // upsert writes the update payload over the stored row
    Write(&'payload BTreeMap<Column, TokenValue>),
}

fn conflict_clause<ClientError, Column>(
    key_name: &str,
    conflict: InsertConflict<'_, Column>,
    parameters: &mut Parameters,
) -> Result<Option<String>, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
    Column: TableColumn,
{
    if let InsertConflict::Write(update) = conflict {
        let mut sets: Vec<String> = bind_payload(update, parameters)?
            .iter()
            .map(|(name, marker)| format!("\"{name}\" = {marker}"))
            .collect();
        if sets.is_empty() {
            sets.push(format!("\"{key_name}\" = EXCLUDED.\"{key_name}\""));
        }
        return Ok(Some(format!(
            " ON CONFLICT (\"{key_name}\") DO UPDATE SET {}",
            sets.join(", "),
        )));
    }
    if matches!(conflict, InsertConflict::Keep) {
        return Ok(Some(format!(
            " ON CONFLICT (\"{key_name}\") DO UPDATE SET \"{key_name}\" = EXCLUDED.\"{key_name}\""
        )));
    }
    Ok(None)
}

pub(super) fn insert<ClientError, Column>(
    create: &BTreeMap<Column, TokenValue>,
    conflict: InsertConflict<'_, Column>,
) -> Result<Statement, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
    Column: TableColumn,
{
    let (key_name, _) = Column::KEY.definition();
    if !create.contains_key(&Column::KEY) {
        return Err(TableOpsError::MissingField { field: key_name });
    }
    let mut parameters = Parameters::default();
    let bound = bind_payload(create, &mut parameters)?;
    let names = bound
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let markers = bound
        .iter()
        .map(|(_, marker)| marker.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let conflict = conflict_clause(key_name, conflict, &mut parameters)?.unwrap_or_default();
    let sql = format!(
        "INSERT INTO \"{}\" AS stored ({names}) VALUES ({markers}){conflict} RETURNING row_to_json(stored)::text",
        Column::TABLE,
    );
    Ok(Statement { sql, parameters })
}

// Prisma's update names the row in its where filter and sets every column of its data payload
pub(super) fn update_where<ClientError, Column>(
    payload: &BTreeMap<Column, TokenValue>,
    key_value: &str,
) -> Result<Statement, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
    Column: TableColumn,
{
    let (key_name, _) = Column::KEY.definition();
    let mut parameters = Parameters::default();
    let key_index = parameters.push(Bound::Text(Some(key_value.to_owned())));
    let bound = bind_payload(payload, &mut parameters)?;
    let where_clause = format!("stored.\"{key_name}\" = ${key_index}");
    let mut sets = Vec::with_capacity(bound.len());
    for (name, marker) in &bound {
        sets.push(format!("\"{name}\" = {marker}"));
    }
    if sets.is_empty() {
        sets.push(format!("\"{key_name}\" = ${key_index}"));
    }
    let sql = format!(
        "UPDATE \"{}\" AS stored SET {} WHERE {where_clause} RETURNING row_to_json(stored)::text",
        Column::TABLE,
        sets.join(", "),
    );
    Ok(Statement { sql, parameters })
}

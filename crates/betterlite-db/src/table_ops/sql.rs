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
    pub(super) fn as_sql(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(value) => value,
            Self::Bool(value) => value,
            Self::Integer(value) => value,
            Self::Array(value) => value,
        }
    }

    fn encode(value: &TokenValue, kind: TokenSqlType) -> Result<Self, serde_json::Error> {
        let bound = match value {
            TokenValue::Text(text) => Self::Text(Some(text.clone())),
            TokenValue::KeyHash(hash) => Self::Text(Some(String::from(hash.expose_secret()))),
            TokenValue::Bool(flag) => Self::Bool(Some(*flag)),
            TokenValue::Integer(number) => Self::Integer(Some(*number)),
            TokenValue::TextArray(items) => Self::Array(Some(items.clone())),
            TokenValue::Number(number) => Self::Text(Some(number.to_string())),
            TokenValue::Json(json) => Self::Text(Some(serde_json::to_string(json)?)),
            TokenValue::Timestamp(timestamp) => Self::Text(Some(
                timestamp
                    .utc()
                    .naive_utc()
                    .format("%Y-%m-%dT%H:%M:%S%.f")
                    .to_string(),
            )),
            TokenValue::Null => match kind {
                TokenSqlType::Boolean => Self::Bool(None),
                TokenSqlType::BigInt => Self::Integer(None),
                TokenSqlType::TextArray => Self::Array(None),
                TokenSqlType::Text
                | TokenSqlType::DoublePrecision
                | TokenSqlType::Json
                | TokenSqlType::Timestamp => Self::Text(None),
            },
        };
        Ok(bound)
    }
}

#[derive(Default)]
pub(super) struct Parameters {
    pub(super) values: Vec<Bound>,
}

impl Parameters {
    fn push(&mut self, bound: Bound) -> usize {
        self.values.push(bound);
        self.values.len()
    }

    fn bind<ClientError>(
        &mut self,
        column: &'static str,
        value: &TokenValue,
        kind: TokenSqlType,
    ) -> Result<String, TableOpsError<ClientError>>
    where
        ClientError: Error + Send + Sync + 'static,
    {
        if !valid_kind(kind, value) {
            return Err(TableOpsError::WrongType { column });
        }
        let bound = Bound::encode(value, kind)
            .map_err(|source| TableOpsError::Encode { column, source })?;
        let cast = match kind {
            TokenSqlType::Text | TokenSqlType::Boolean | TokenSqlType::TextArray => "",
            TokenSqlType::BigInt => "::bigint",
            TokenSqlType::DoublePrecision => "::text::double precision",
            TokenSqlType::Json => "::text::jsonb",
            TokenSqlType::Timestamp => "::text::timestamp(3)",
        };
        let index = self.push(bound);
        Ok(format!("${index}{cast}"))
    }
}

pub(super) struct Statement {
    pub(super) sql: String,
    pub(super) parameters: Parameters,
}

const fn valid_kind(kind: TokenSqlType, value: &TokenValue) -> bool {
    if matches!(value, TokenValue::Null) {
        return true;
    }
    match kind {
        TokenSqlType::Text => matches!(value, TokenValue::Text(_) | TokenValue::KeyHash(_)),
        TokenSqlType::Boolean => matches!(value, TokenValue::Bool(_)),
        TokenSqlType::BigInt => matches!(value, TokenValue::Integer(_)),
        TokenSqlType::TextArray => matches!(value, TokenValue::TextArray(_)),
        TokenSqlType::DoublePrecision | TokenSqlType::Json | TokenSqlType::Timestamp => matches!(
            value,
            TokenValue::Number(_)
                | TokenValue::Json(_)
                | TokenValue::Timestamp(_)
                | TokenValue::Text(_)
        ),
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
    payload
        .iter()
        .map(|(column, value)| {
            let (name, kind) = column.sql_definition();
            parameters
                .bind(name, value, kind)
                .map(|marker| (name, marker))
        })
        .collect()
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

// Parameter binding finishes before formatting the conflict clause.
fn conflict_clause(key_name: &str, bound: Option<&[(&'static str, String)]>) -> String {
    let Some(columns) = bound else {
        return String::new();
    };
    let mut assignments: Vec<String> = columns
        .iter()
        .map(|(name, marker)| format!("\"{name}\" = {marker}"))
        .collect();
    if assignments.is_empty() {
        assignments.push(format!("\"{key_name}\" = EXCLUDED.\"{key_name}\""));
    }
    format!(
        " ON CONFLICT (\"{key_name}\") DO UPDATE SET {}",
        assignments.join(", "),
    )
}

pub(super) fn insert<ClientError, Column>(
    create: &BTreeMap<Column, TokenValue>,
    conflict: InsertConflict<'_, Column>,
) -> Result<Statement, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
    Column: TableColumn,
{
    let (key_name, _) = Column::KEY.sql_definition();
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
    let update = match conflict {
        InsertConflict::Reject => None,
        InsertConflict::Keep => Some(Vec::new()),
        InsertConflict::Write(values) => Some(bind_payload(values, &mut parameters)?),
    };
    let conflict_sql = conflict_clause(key_name, update.as_deref());
    let sql = format!(
        "INSERT INTO \"{}\" AS stored ({names}) VALUES ({markers}){conflict_sql} RETURNING row_to_json(stored)::text",
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
    let (key_name, _) = Column::KEY.sql_definition();
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

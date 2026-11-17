use std::collections::BTreeMap;

use deadpool_postgres::GenericClient;
use secrecy::ExposeSecret as _;
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::JsonData;
use crate::key_repository::table::columns::{TokenColumn, TokenSqlType};
use crate::key_repository::table::{TokenValue, TokenWrite};
use crate::postgres_store::errors::backend;

use super::columns::UserWrite;
use super::rows;
use super::{KeyUserWriteError, WriteRowError};

const TOKEN_TABLE: &str = "\"LiteLLM_VerificationToken\"";
const USER_TABLE: &str = "\"LiteLLM_UserTable\"";
const BUDGET_TABLE: &str = "\"LiteLLM_BudgetTable\"";
const PERMISSION_TABLE: &str = "\"LiteLLM_ObjectPermissionTable\"";
const ROW_JSON: &str = "row_to_json(r)::text";
const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.f";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Relation {
    Budget,
    ObjectPermission,
}

impl Relation {
    const fn id_field(self) -> &'static str {
        match self {
            Self::Budget => "budget_id",
            Self::ObjectPermission => "object_permission_id",
        }
    }

    const fn target_field(self) -> &'static str {
        match self {
            Self::Budget => "litellm_budget_table",
            Self::ObjectPermission => "object_permission",
        }
    }

    const fn table(self) -> &'static str {
        match self {
            Self::Budget => BUDGET_TABLE,
            Self::ObjectPermission => PERMISSION_TABLE,
        }
    }
}

pub(super) enum Parameter {
    Text(Option<String>),
    Array(Option<Vec<String>>),
}

impl Parameter {
    pub(super) fn bind(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(value) => value,
            Self::Array(value) => value,
        }
    }

    fn marker(&self, index: usize, sql_type: TokenSqlType) -> String {
        match self {
            Self::Array(_) => format!("${index}::text[]"),
            Self::Text(_) => match sql_type {
                TokenSqlType::Text => format!("${index}::text"),
                TokenSqlType::Boolean => format!("${index}::text::boolean"),
                TokenSqlType::BigInt => format!("${index}::text::bigint"),
                TokenSqlType::DoublePrecision => format!("${index}::text::double precision"),
                TokenSqlType::Json => format!("${index}::text::jsonb"),
                TokenSqlType::Timestamp => format!("${index}::text::timestamp"),
                TokenSqlType::TextArray => format!("${index}::text[]"),
            },
        }
    }
}

#[derive(Default)]
pub(super) struct Statement {
    pub(super) parameters: Vec<Parameter>,
}

pub(super) fn upsert_key(
    statement: &mut Statement,
    fields: &TokenWrite,
) -> Result<String, KeyUserWriteError> {
    let (columns, placeholders) = bind_values(
        statement,
        fields
            .0
            .iter()
            .map(|(column, value)| (column.name(), column.sql_type(), value)),
    )?;
    Ok(format!(
        "INSERT INTO {TOKEN_TABLE} AS r ({}) VALUES ({}) ON CONFLICT (\"token\") DO UPDATE SET \"token\" = EXCLUDED.\"token\" RETURNING {ROW_JSON}",
        columns.join(", "),
        placeholders.join(", ")
    ))
}

pub(super) fn upsert_user(
    statement: &mut Statement,
    create: &UserWrite,
    update: &UserWrite,
) -> Result<String, KeyUserWriteError> {
    let (columns, placeholders) = bind_values(statement, create.bindings())?;
    let (update_columns, update_placeholders) = bind_values(statement, update.bindings())?;
    let sets = assignments(&update_columns, &update_placeholders);
    let conflict = if sets.is_empty() {
        String::from("\"user_id\" = EXCLUDED.\"user_id\"")
    } else {
        sets.join(", ")
    };
    Ok(format!(
        "INSERT INTO {USER_TABLE} AS r ({}) VALUES ({}) ON CONFLICT (\"user_id\") DO UPDATE SET {conflict} RETURNING {ROW_JSON}",
        columns.join(", "),
        placeholders.join(", ")
    ))
}

pub(super) fn update_key(
    statement: &mut Statement,
    fields: &TokenWrite,
) -> Result<String, KeyUserWriteError> {
    let (columns, placeholders) = bind_values(
        statement,
        fields
            .0
            .iter()
            .map(|(column, value)| (column.name(), column.sql_type(), value)),
    )?;
    let sets = assignments(&columns, &placeholders);
    let token = fields
        .0
        .get(&TokenColumn::Token)
        .ok_or(KeyUserWriteError::MissingField { field: "token" })?;
    let parameter = parameter_for(TokenSqlType::Text, token)?;
    let index = statement.parameters.len().saturating_add(1);
    let marker = parameter.marker(index, TokenSqlType::Text);
    statement.parameters.push(parameter);
    Ok(format!(
        "UPDATE {TOKEN_TABLE} AS r SET {} WHERE r.\"token\" = {marker} RETURNING {ROW_JSON}",
        sets.join(", ")
    ))
}

fn bind_values<'row, Fields>(
    statement: &mut Statement,
    fields: Fields,
) -> Result<(Vec<String>, Vec<String>), KeyUserWriteError>
where
    Fields: IntoIterator<Item = (&'static str, TokenSqlType, &'row TokenValue)>,
{
    let mut columns = Vec::new();
    let mut placeholders = Vec::new();
    for (name, sql_type, value) in fields {
        let index = statement.parameters.len().saturating_add(1);
        let parameter = parameter_for(sql_type, value)?;
        placeholders.push(parameter.marker(index, sql_type));
        statement.parameters.push(parameter);
        columns.push(format!("\"{name}\""));
    }
    Ok((columns, placeholders))
}

fn assignments(columns: &[String], placeholders: &[String]) -> Vec<String> {
    let mut sets = Vec::with_capacity(columns.len());
    for (column, marker) in columns.iter().zip(placeholders) {
        sets.push(format!("{column} = {marker}"));
    }
    sets
}

fn parameter_for(
    sql_type: TokenSqlType,
    value: &TokenValue,
) -> Result<Parameter, KeyUserWriteError> {
    match value {
        TokenValue::TextArray(items) => Ok(Parameter::Array(Some(items.clone()))),
        TokenValue::Null => Ok(null_parameter(sql_type)),
        TokenValue::KeyHash(key) => Ok(Parameter::Text(Some(key.expose_secret().to_owned()))),
        TokenValue::Text(text) => Ok(Parameter::Text(Some(text.clone()))),
        TokenValue::Json(json) => Ok(Parameter::Text(Some(
            serde_json::to_string(json).map_err(|source| KeyUserWriteError::Encode { source })?,
        ))),
        TokenValue::Timestamp(timestamp) => Ok(Parameter::Text(Some(
            timestamp
                .utc()
                .naive_utc()
                .format(TIMESTAMP_FORMAT)
                .to_string(),
        ))),
        TokenValue::Bool(flag) => Ok(Parameter::Text(Some(String::from(if *flag {
            "true"
        } else {
            "false"
        })))),
        TokenValue::Integer(number) => Ok(Parameter::Text(Some(number.to_string()))),
        TokenValue::Number(number) => Ok(Parameter::Text(Some(number.to_string()))),
    }
}

const fn null_parameter(sql_type: TokenSqlType) -> Parameter {
    match sql_type {
        TokenSqlType::TextArray => Parameter::Array(None),
        TokenSqlType::Text
        | TokenSqlType::Boolean
        | TokenSqlType::BigInt
        | TokenSqlType::DoublePrecision
        | TokenSqlType::Json
        | TokenSqlType::Timestamp => Parameter::Text(None),
    }
}

pub(super) async fn attach_relation<C>(
    client: &C,
    fields: &mut BTreeMap<String, JsonData>,
    relation: Relation,
) -> Result<(), KeyUserWriteError>
where
    C: GenericClient + Sync,
{
    let row_value = match relation_id(fields, relation.id_field()) {
        Some(id_value) => query_relation(client, relation, &id_value).await?,
        None => JsonData::Null,
    };
    fields.insert(String::from(relation.target_field()), row_value);
    Ok(())
}

async fn query_relation<C>(
    client: &C,
    relation: Relation,
    id_value: &str,
) -> Result<JsonData, KeyUserWriteError>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT {ROW_JSON} FROM {} AS r WHERE r.\"{}\" = $1",
        relation.table(),
        relation.id_field()
    );
    let row = client
        .query_opt(sql.as_str(), &[&id_value])
        .await
        .map_err(|driver| KeyUserWriteError::Database {
            source: backend::<WriteRowError>(driver),
        })?;
    match row {
        Some(found) => rows::row_json(&found),
        None => Ok(JsonData::Null),
    }
}

fn relation_id(fields: &BTreeMap<String, JsonData>, name: &str) -> Option<String> {
    match fields.get(name) {
        Some(JsonData::String(value)) => Some(value.clone()),
        _ => None,
    }
}

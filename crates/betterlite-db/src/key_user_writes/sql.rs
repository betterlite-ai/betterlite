use std::collections::BTreeMap;

use deadpool_postgres::GenericClient;
use secrecy::ExposeSecret as _;
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::JsonData;
use crate::key_repository::table::columns::{TokenColumn, TokenSqlType};
use crate::key_repository::table::{TokenValue, TokenWrite};
use crate::postgres_store::errors::backend;

use super::assignments::{assignments, update_assignments};
use super::columns::{UserColumn, UserUpdateOps, UserWrite};
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

    pub(super) fn marker(&self, index: usize, sql_type: TokenSqlType) -> String {
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
    let mut insert = fields.0.clone();
    insert
        .entry(TokenColumn::Models)
        .or_insert_with(|| TokenValue::TextArray(Vec::new()));
    let (columns, placeholders) = bind_values(
        statement,
        insert
            .iter()
            .map(|(column, value)| (column.name(), column.sql_type(), value)),
    )?;
    Ok(format!(
        "INSERT INTO {TOKEN_TABLE} AS r ({}) VALUES ({}) ON CONFLICT (\"token\") DO UPDATE SET \"token\" = EXCLUDED.\"token\" RETURNING {ROW_JSON}",
        columns.join(", "),
        placeholders.join(", ")
    ))
}

#[derive(Clone, Copy)]
pub(super) enum UserConflict<'update> {
    Reject,
    Upsert(&'update UserUpdateOps),
}

pub(super) fn insert_user(
    statement: &mut Statement,
    create: &UserWrite,
    conflict: UserConflict<'_>,
) -> Result<String, KeyUserWriteError> {
    let (columns, placeholders) = insert_user_values(statement, create)?;
    let insert = format!(
        "INSERT INTO {USER_TABLE} AS r ({}) VALUES ({})",
        columns.join(", "),
        placeholders.join(", ")
    );
    match conflict {
        UserConflict::Reject => Ok(format!("{insert} RETURNING {ROW_JSON}")),
        UserConflict::Upsert(update) => {
            let sets = update_set(statement, update, "\"user_id\" = EXCLUDED.\"user_id\"")?;
            Ok(format!(
                "{insert} ON CONFLICT (\"user_id\") DO UPDATE SET {sets} RETURNING {ROW_JSON}"
            ))
        }
    }
}

fn update_set(
    statement: &mut Statement,
    update: &UserUpdateOps,
    unchanged: &str,
) -> Result<String, KeyUserWriteError> {
    if update.0.is_empty() {
        return Ok(unchanged.to_owned());
    }
    Ok(update_assignments(statement, update)?.join(", "))
}

pub(super) fn update_user(
    statement: &mut Statement,
    lookup: &str,
    update: &UserUpdateOps,
) -> Result<String, KeyUserWriteError> {
    let sets = update_set(statement, update, "\"user_id\" = r.\"user_id\"")?;
    let index = statement.parameters.len().saturating_add(1);
    statement
        .parameters
        .push(Parameter::Text(Some(lookup.to_owned())));
    Ok(format!(
        "UPDATE {USER_TABLE} AS r SET {sets} WHERE r.\"user_id\" = ${index}::text RETURNING {ROW_JSON}"
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

fn insert_user_values(
    statement: &mut Statement,
    create: &UserWrite,
) -> Result<(Vec<String>, Vec<String>), KeyUserWriteError> {
    let mut insert = create.clone();
    insert
        .0
        .entry(UserColumn::Models)
        .or_insert_with(|| TokenValue::TextArray(Vec::new()));
    bind_values(statement, insert.bindings())
}

pub(super) fn parameter_for(
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
    let row_value = match rows::relation_id(fields, relation.id_field()) {
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
    row.map_or_else(|| Ok(JsonData::Null), |found| rows::row_json(&found))
}

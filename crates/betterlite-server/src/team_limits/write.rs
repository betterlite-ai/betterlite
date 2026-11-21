use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::key_repository::table::TokenValue;
use betterlite_db::key_repository::table::columns::TokenSqlType;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::table_ops::columns::{TableColumn as _, TeamColumn};
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use crate::team_limits::guard::MaxBudgetGuard;

// find_unique with both relations included; a scalar subquery keeps an absent relation SQL null, where a left join gives an object of nulls
const REREAD_SQL: &str = r#"SELECT row_to_json(x)::text FROM (SELECT t.*, (SELECT to_jsonb(m.*) FROM "LiteLLM_ModelTable" AS m WHERE m."id" = t."model_id") AS litellm_model_table, (SELECT to_jsonb(p.*) FROM "LiteLLM_ObjectPermissionTable" AS p WHERE p."object_permission_id" = t."object_permission_id") AS object_permission FROM "LiteLLM_TeamTable" AS t WHERE t."team_id" = $1) x"#;

#[derive(Debug, thiserror::Error)]
pub enum TeamWriteError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("The team's max_budget changed during this update. Reload the team and try again.")]
    MaxBudgetChanged,
    #[error("the updated team row cannot be decoded")]
    Decode(#[from] TeamRecordError),
    #[error("the write value for {column} cannot be encoded")]
    Encode {
        column: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

impl TeamWriteError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::MaxBudgetChanged => StatusCode::CONFLICT,
            Self::Database(_) | Self::Decode(_) | Self::Encode { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

enum WriteParameter {
    Text(Option<String>),
    Array(Option<Vec<String>>),
}

impl WriteParameter {
    fn as_sql(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(value) => value,
            Self::Array(value) => value,
        }
    }
}

pub async fn write_team_update(
    store: &PostgresStore,
    team_id: &str,
    data: &BTreeMap<TeamColumn, TokenValue>,
    guard: Option<&MaxBudgetGuard>,
    now: DateTime<Utc>,
) -> Result<Option<LiteLLMTeamTable>, TeamWriteError> {
    let mut parameters = vec![WriteParameter::Text(Some(team_id.to_owned()))];
    let assignments = update_assignments(data, now, &mut parameters)?;
    let conditions = update_conditions(guard, &mut parameters);
    let statement = format!(
        "UPDATE \"LiteLLM_TeamTable\" SET {} WHERE {}",
        assignments.join(", "),
        conditions.join(" AND ")
    );
    let bound: Vec<&(dyn ToSql + Sync)> = parameters.iter().map(WriteParameter::as_sql).collect();
    let client = store.session().await?;
    let written = client.execute(&statement, &bound).await.map_err(backend)?;
    if guard.is_some() && written == 0 {
        return Err(TeamWriteError::MaxBudgetChanged);
    }
    read_back(&client, team_id).await
}

fn update_assignments(
    data: &BTreeMap<TeamColumn, TokenValue>,
    now: DateTime<Utc>,
    parameters: &mut Vec<WriteParameter>,
) -> Result<Vec<String>, TeamWriteError> {
    let mut payload = data.clone();
    // Prisma's @updatedAt stamps every update that does not set it explicitly
    let stamped = TokenValue::Timestamp(Timestamp::Naive(now.naive_utc()));
    payload.entry(TeamColumn::UpdatedAt).or_insert(stamped);
    let mut assignments = Vec::with_capacity(payload.len());
    for (column, value) in &payload {
        let (parameter, cast) = encode(*column, value)?;
        let index = parameters.len().saturating_add(1);
        let (name, _) = column.sql_definition();
        assignments.push(format!("\"{name}\" = ${index}{cast}"));
        parameters.push(parameter);
    }
    Ok(assignments)
}

fn update_conditions(
    guard: Option<&MaxBudgetGuard>,
    parameters: &mut Vec<WriteParameter>,
) -> Vec<String> {
    let mut conditions = vec![String::from("\"team_id\" = $1")];
    if let Some(cap_guard) = guard {
        let index = parameters.len().saturating_add(1);
        parameters.push(WriteParameter::Text(
            cap_guard.expected.map(|amount| amount.0.to_string()),
        ));
        conditions.push(format!(
            "\"max_budget\" IS NOT DISTINCT FROM ${index}::text::double precision"
        ));
    }
    conditions
}

async fn read_back(
    client: &Client,
    team_id: &str,
) -> Result<Option<LiteLLMTeamTable>, TeamWriteError> {
    let parameter: [&(dyn ToSql + Sync); 1] = [&team_id];
    let rows = client
        .query(REREAD_SQL, &parameter)
        .await
        .map_err(backend)?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let stored: String = row.try_get(0).map_err(backend)?;
    let record = TeamRecord::parse(stored.as_bytes())?;
    decode_team_record(record.as_ref()).map_err(TeamWriteError::from)
}

fn encode(
    column: TeamColumn,
    value: &TokenValue,
) -> Result<(WriteParameter, &'static str), TeamWriteError> {
    let (name, sql_type) = column.sql_definition();
    let cast = cast_text(sql_type);
    let parameter = match value {
        TokenValue::Text(text) => WriteParameter::Text(Some(text.clone())),
        TokenValue::KeyHash(key) => WriteParameter::Text(Some(String::from(key.expose_secret()))),
        TokenValue::Bool(flag) => {
            WriteParameter::Text(Some(String::from(if *flag { "true" } else { "false" })))
        }
        TokenValue::Integer(number) => WriteParameter::Text(Some(number.to_string())),
        TokenValue::Number(number) => WriteParameter::Text(Some(number.to_string())),
        TokenValue::Json(json) => WriteParameter::Text(Some(serialize_json(name, json)?)),
        TokenValue::Timestamp(timestamp) => WriteParameter::Text(Some(timestamp_text(*timestamp))),
        TokenValue::TextArray(items) => WriteParameter::Array(Some(items.clone())),
        TokenValue::Null => null_parameter(sql_type),
    };
    Ok((parameter, cast))
}

fn serialize_json(name: &'static str, json: &JsonData) -> Result<String, TeamWriteError> {
    serde_json::to_string(json).map_err(|source| TeamWriteError::Encode {
        column: name,
        source,
    })
}

const fn null_parameter(sql_type: TokenSqlType) -> WriteParameter {
    match sql_type {
        TokenSqlType::TextArray => WriteParameter::Array(None),
        TokenSqlType::Text
        | TokenSqlType::Boolean
        | TokenSqlType::BigInt
        | TokenSqlType::DoublePrecision
        | TokenSqlType::Json
        | TokenSqlType::Timestamp => WriteParameter::Text(None),
    }
}

fn timestamp_text(timestamp: Timestamp) -> String {
    timestamp
        .utc()
        .naive_utc()
        .format("%Y-%m-%dT%H:%M:%S%.f")
        .to_string()
}

const fn cast_text(sql_type: TokenSqlType) -> &'static str {
    match sql_type {
        TokenSqlType::Text => "::text",
        TokenSqlType::Boolean => "::text::boolean",
        TokenSqlType::BigInt => "::text::bigint",
        TokenSqlType::DoublePrecision => "::text::double precision",
        TokenSqlType::Json => "::text::jsonb",
        TokenSqlType::Timestamp => "::text::timestamp",
        TokenSqlType::TextArray => "::text[]",
    }
}

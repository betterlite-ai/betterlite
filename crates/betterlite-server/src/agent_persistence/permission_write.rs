use std::collections::BTreeMap;

use betterlite_core::agents::permissions::AgentObjectPermission;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::DbSession;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::object_permissions::prepare::PreparedUpsert;
// ast-grep-ignore: no-long-names
use crate::object_permissions::prepare::prepare_object_permission_upsert;

use super::error::AgentPersistenceError;
use super::row::ColumnList;
use super::row::execute;

const PERMISSION_TABLE: &str = "LiteLLM_ObjectPermissionTable";

const COLUMNS: [&str; 13] = [
    "object_permission_id",
    "mcp_servers",
    "mcp_access_groups",
    "mcp_tool_permissions",
    "vector_stores",
    "agents",
    "agent_access_groups",
    "models",
    "blocked_tools",
    "mcp_toolsets",
    "search_tools",
    "mcp_tool_search_enabled",
    "skills",
];

enum ColumnKind {
    List,
    Json,
    Flag,
}

enum PermissionValue {
    List(Option<Vec<String>>),
    Json(String),
    Flag(Option<bool>),
}

fn kind_of(name: &str) -> Option<ColumnKind> {
    match name {
        "mcp_tool_permissions" => Some(ColumnKind::Json),
        "mcp_tool_search_enabled" => Some(ColumnKind::Flag),
        "mcp_servers"
        | "mcp_access_groups"
        | "vector_stores"
        | "agents"
        | "agent_access_groups"
        | "models"
        | "blocked_tools"
        | "mcp_toolsets"
        | "search_tools"
        | "skills" => Some(ColumnKind::List),
        _ => None,
    }
}

fn convert(value: &JsonData, kind: ColumnKind) -> Result<PermissionValue, AgentPersistenceError> {
    match kind {
        ColumnKind::List => Ok(PermissionValue::List(serde_json::from_value(
            serde_json::to_value(value)?,
        )?)),
        ColumnKind::Json => Ok(PermissionValue::Json(serde_json::to_string(value)?)),
        ColumnKind::Flag => Ok(PermissionValue::Flag(serde_json::from_value(
            serde_json::to_value(value)?,
        )?)),
    }
}

fn converted_record(
    record: &BTreeMap<String, JsonData>,
) -> Result<Vec<(&'static str, PermissionValue)>, AgentPersistenceError> {
    let mut converted = Vec::new();
    for name in COLUMNS {
        let Some(kind) = kind_of(name) else {
            continue;
        };
        let Some(value) = record.get(name) else {
            continue;
        };
        converted.push((name, convert(value, kind)?));
    }
    Ok(converted)
}

pub(super) fn permission_fields(
    permission: &AgentObjectPermission,
) -> Result<BTreeMap<String, JsonData>, AgentPersistenceError> {
    let document = serde_json::from_value::<JsonData>(serde_json::to_value(permission)?)?;
    let JsonData::Object(fields) = document else {
        return Ok(BTreeMap::new());
    };
    Ok(fields)
}

pub(super) async fn prepare(
    store: &PostgresStore,
    permission: Option<&AgentObjectPermission>,
    existing_id: Option<&str>,
) -> Result<Option<PreparedUpsert>, AgentPersistenceError> {
    let Some(permission) = permission else {
        return Ok(None);
    };
    let fields = permission_fields(permission)?;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: store,
        reader: store,
        availability: &availability,
    };
    // ast-grep-ignore: no-long-names
    let prepared = prepare_object_permission_upsert(&fields, existing_id, client, &[]).await?;
    Ok(Some(prepared))
}

pub(super) async fn write_permission(
    tx: &DbSession<'_>,
    prepared: &PreparedUpsert,
) -> Result<(), AgentPersistenceError> {
    let converted = converted_record(&prepared.record)?;
    let mut list = ColumnList::default();
    list.push("object_permission_id", &prepared.object_permission_id, "");
    for (name, value) in &converted {
        match value {
            PermissionValue::List(values) => list.push(*name, values, ""),
            PermissionValue::Json(text) => list.push(*name, text, "::text::jsonb"),
            PermissionValue::Flag(flag) => list.push(*name, flag, ""),
        }
    }
    let mut assignments: Vec<String> = list
        .quoted_names()
        .iter()
        .skip(1)
        .map(|column| format!("{column} = EXCLUDED.{column}"))
        .collect();
    if assignments.is_empty() {
        assignments.push(String::from(
            "\"object_permission_id\" = EXCLUDED.\"object_permission_id\"",
        ));
    }
    let sql = format!(
        "{} ON CONFLICT (\"object_permission_id\") DO UPDATE SET {}",
        list.insert_sql(PERMISSION_TABLE),
        assignments.join(", "),
    );
    execute(tx, &sql, &list.parameters).await?;
    Ok(())
}

mod tests;

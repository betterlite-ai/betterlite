use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::rows::RowFields;
use betterlite_db::mcp::table_access::server_reads::{
    _identifier_field, get_mcp_server, get_mcp_servers,
};
use betterlite_db::mcp::table_access::{
    _mcp_identifier_lock_keys, approve_mcp_server, find_identifier_conflict, get_all_mcp_servers,
    get_mcp_submissions, reject_mcp_server, set_mcp_server_pinned_tools,
};
// ast-grep-ignore: no-long-names
use betterlite_db::mcp::table_access::get_active_submitted_mcp_server_ids_for_user as get_active_submitted_ids;
use betterlite_db::mcp_stored_contracts::{JsonValue, PinnedMCPTool};
use serde_json::{Value, json};

use crate::helpers::HelperError;

use super::ReviewAction;
use super::helper_error;
use super::rows::RowStore;
use super::views::{cipher, conflict_value, review_view, server_view};

pub(super) async fn find_conflict(
    server_name: Option<&str>,
    alias: Option<&str>,
    exclude_server_id: Option<&str>,
    rows: &[Value],
) -> Result<Value, Box<HelperError>> {
    let store = store_from(rows)?;
    let conflict = find_identifier_conflict(&store, server_name, alias, exclude_server_id)
        .await
        .map_err(|source| helper_error("McpTableError", &source.to_string()))?;
    Ok(json!({"conflict": conflict_value(conflict)}))
}

pub(super) async fn create_if_free(
    server_name: &Value,
    alias: &Value,
    rows: &[Value],
) -> Result<Value, Box<HelperError>> {
    let store = store_from(rows)?;
    let fields = identifier_fields(Some(server_name), Some(alias))?;
    let extracted_name = _identifier_field(&fields, "server_name");
    let extracted_alias = _identifier_field(&fields, "alias");
    let lock_keys =
        _mcp_identifier_lock_keys(&[extracted_name.as_deref(), extracted_alias.as_deref()]);
    let conflict = find_identifier_conflict(
        &store,
        extracted_name.as_deref(),
        extracted_alias.as_deref(),
        None,
    )
    .await
    .map_err(|source| helper_error("McpTableError", &source.to_string()))?;
    let written = conflict.is_none();
    Ok(json!({
        "conflict": conflict_value(conflict),
        "lock_keys": lock_keys,
        "written": written,
    }))
}

pub(super) async fn get_all(
    salt_key: &str,
    approval_status: Option<&str>,
    server_ids: Option<&[String]>,
    rows: &[Value],
) -> Result<Value, Box<HelperError>> {
    let store = store_from(rows)?;
    let db_cipher = cipher(salt_key);
    let servers = match server_ids {
        Some(server_ids) => get_mcp_servers(&store, &db_cipher, server_ids)
            .await
            .map_err(|source| helper_error("McpTableError", &source.to_string()))?,
        None => get_all_mcp_servers(&store, &db_cipher, approval_status)
            .await
            .map_err(|source| helper_error("McpTableError", &source.to_string()))?,
    };
    let mut views = Vec::with_capacity(servers.len());
    for server in &servers {
        views.push(server_view(server)?);
    }
    Ok(json!({"servers": views}))
}

pub(super) async fn get_server(
    salt_key: &str,
    server_id: &str,
    rows: &[Value],
) -> Result<Value, Box<HelperError>> {
    let store = store_from(rows)?;
    let found = get_mcp_server(&store, &cipher(salt_key), server_id)
        .await
        .map_err(|source| helper_error("McpTableError", &source.to_string()))?;
    let Some(server) = found else {
        return Ok(json!({"server": Value::Null}));
    };
    let view = server_view(&server)?;
    Ok(json!({"server": view}))
}

pub(super) async fn review(
    salt_key: &str,
    action: &ReviewAction,
    server_id: &str,
    touched_by: &str,
    review_notes: Option<&str>,
    rows: &[Value],
) -> Result<Value, Box<HelperError>> {
    let store = store_from(rows)?;
    let db_cipher = cipher(salt_key);
    let server = match action {
        ReviewAction::Approve => {
            approve_mcp_server(&store, &db_cipher, server_id, touched_by).await
        }
        ReviewAction::Reject => {
            reject_mcp_server(&store, &db_cipher, server_id, touched_by, review_notes).await
        }
    }
    .map_err(|source| helper_error("McpTableError", &source.to_string()))?;
    Ok(review_view(&server))
}

pub(super) async fn pinned_tools(
    salt_key: &str,
    server_id: &str,
    pinned_tools: Option<&Value>,
    touched_by: &str,
    rows: &[Value],
) -> Result<Value, Box<HelperError>> {
    let store = store_from(rows)?;
    let decoded: Option<BTreeMap<String, PinnedMCPTool>> = pinned_tools
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|source| helper_error("InvalidInput", &source.to_string()))?;
    let updated = set_mcp_server_pinned_tools(
        &store,
        &cipher(salt_key),
        server_id,
        decoded.as_ref(),
        touched_by,
    )
    .await
    .map_err(|source| helper_error("McpTableError", &source.to_string()))?;
    let Some(server) = updated else {
        return Ok(json!({"found": false, "pinned_tools": Value::Null}));
    };
    let tools = serde_json::to_value(&server.pinned_tools)
        .map_err(|source| helper_error("SerializationError", &source.to_string()))?;
    Ok(json!({"found": true, "pinned_tools": tools}))
}

pub(super) async fn submissions(salt_key: &str, rows: &[Value]) -> Result<Value, Box<HelperError>> {
    let store = store_from(rows)?;
    let summary = get_mcp_submissions(&store, &cipher(salt_key))
        .await
        .map_err(|source| helper_error("McpTableError", &source.to_string()))?;
    let first_server_id = summary
        .items
        .first()
        .map(|server| server.server_id.as_str());
    let last_server_id = summary.items.last().map(|server| server.server_id.as_str());
    Ok(json!({
        "total": summary.total,
        "pending_review": summary.pending_review,
        "active": summary.active,
        "rejected": summary.rejected,
        "first_server_id": first_server_id,
        "last_server_id": last_server_id,
    }))
}

pub(super) async fn active_submissions(
    user_id: &str,
    rows: &[Value],
) -> Result<Value, Box<HelperError>> {
    let store = store_from(rows)?;
    let server_ids = get_active_submitted_ids(&store, user_id)
        .await
        .map_err(|source| helper_error("McpTableError", &source.to_string()))?;
    Ok(json!({"server_ids": server_ids}))
}

fn store_from(rows: &[Value]) -> Result<RowStore, Box<HelperError>> {
    let mut stored = Vec::with_capacity(rows.len());
    for row in rows {
        stored.push(row_data(row)?);
    }
    Ok(RowStore {
        rows: Mutex::new(stored),
    })
}

fn row_data(value: &Value) -> Result<JsonData, Box<HelperError>> {
    if !value.is_object() {
        return Err(helper_error(
            "InvalidInput",
            "every row must be a JSON object",
        ));
    }
    serde_json::from_value(value.clone())
        .map_err(|source| helper_error("InvalidInput", &source.to_string()))
}

fn identifier_fields(
    server_name: Option<&Value>,
    alias: Option<&Value>,
) -> Result<RowFields, Box<HelperError>> {
    let mut fields: BTreeMap<&'static str, JsonValue> = BTreeMap::new();
    for (name, value) in [("server_name", server_name), ("alias", alias)] {
        let Some(value) = value else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let decoded = serde_json::from_value(value.clone())
            .map_err(|source| helper_error("InvalidInput", &source.to_string()))?;
        fields.insert(name, decoded);
    }
    Ok(RowFields(fields))
}

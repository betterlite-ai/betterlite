use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::rows::RowFields;
use betterlite_db::mcp::table_access::server_reads::{
    get_mcp_server, get_mcp_servers, identifier_field,
};
use betterlite_db::mcp::table_access::{
    McpIdentifierConflict, approve_mcp_server, find_identifier_conflict, get_all_mcp_servers,
    get_mcp_submissions, mcp_identifier_lock_keys, reject_mcp_server, set_mcp_server_pinned_tools,
};
// ast-grep-ignore: no-long-names
use betterlite_db::mcp::table_access::get_active_submitted_mcp_server_ids_for_user as active_ids;
use betterlite_db::mcp_stored_contracts::{JsonValue, PinnedMCPTool};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::HelperResult;
use crate::helpers::OrHelperError as _;

use super::rows::RowStore;
use super::{
    ActiveSubmissions, CreateIfFree, FindConflict, GetAll, GetServer, PinnedTools, Review,
    ReviewAction, Submissions,
};

fn cipher(salt: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn conflict_value(conflict: Option<&McpIdentifierConflict>) -> Value {
    conflict.map_or(Value::Null, |found| {
        json!({"field": found.field.as_str(), "value": found.value, "server_id": found.server_id})
    })
}

fn server_view(server: &McpServerTable) -> HelperResult {
    let env = serde_json::to_value(&server.env).or_value_error()?;
    Ok(
        json!({"server_id": server.server_id, "approval_status": server.approval_status, "env": env}),
    )
}

pub(super) async fn find_conflict(call: &FindConflict) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let found = find_identifier_conflict(
        &store,
        call.server_name.as_deref(),
        call.alias.as_deref(),
        call.exclude_server_id.as_deref(),
    )
    .await
    .or_value_error()?;
    Ok(conflict_value(found.as_ref()))
}

// the create path reads both identifiers back as strings, locks them, then checks for a collision
pub(super) async fn create_if_free(call: &CreateIfFree) -> HelperResult {
    let mut fields = BTreeMap::new();
    for (name, value) in [("server_name", &call.server_name), ("alias", &call.alias)] {
        fields.insert(
            name,
            value.clone().map_or(JsonValue::Null, JsonValue::String),
        );
    }
    let data = RowFields(fields);
    let name = identifier_field(&data, "server_name");
    let alias = identifier_field(&data, "alias");
    let lock_keys = mcp_identifier_lock_keys(&[name.as_deref(), alias.as_deref()]);
    let store = RowStore::new(&call.rows);
    let conflict = find_identifier_conflict(&store, name.as_deref(), alias.as_deref(), None)
        .await
        .or_value_error()?;
    Ok(json!({
        "conflict": conflict_value(conflict.as_ref()),
        "lock_keys": lock_keys,
        "written": conflict.is_none(),
    }))
}

pub(super) async fn get_all(call: &GetAll) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let db_cipher = cipher(&call.salt_key);
    let servers = match &call.server_ids {
        Some(ids) => get_mcp_servers(&store, &db_cipher, ids).await,
        None => get_all_mcp_servers(&store, &db_cipher, call.approval_status.as_deref()).await,
    }
    .or_value_error()?;
    let mut views = Vec::with_capacity(servers.len());
    for server in &servers {
        views.push(server_view(server)?);
    }
    Ok(Value::Array(views))
}

pub(super) async fn get_server(call: &GetServer) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let read = get_mcp_server(&store, &cipher(&call.salt_key), &call.server_id).await;
    read.or_value_error()?
        .as_ref()
        .map_or(Ok(Value::Null), server_view)
}

pub(super) async fn review(call: &Review) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let db_cipher = cipher(&call.salt_key);
    let (server_id, touched_by) = (call.server_id.as_str(), call.touched_by.as_str());
    let reviewed = match call.action {
        ReviewAction::Approve => {
            approve_mcp_server(&store, &db_cipher, server_id, touched_by).await
        }
        ReviewAction::Reject => {
            let notes = call.review_notes.as_deref();
            reject_mcp_server(&store, &db_cipher, server_id, touched_by, notes).await
        }
    };
    let server = reviewed.or_value_error()?;
    Ok(json!({
        "server_id": server.server_id,
        "approval_status": server.approval_status,
        "review_notes": server.review_notes,
        "updated_by": server.updated_by,
        "reviewed_at_present": server.reviewed_at.is_some(),
    }))
}

pub(super) async fn pinned_tools(call: &PinnedTools) -> HelperResult {
    let tools: Option<BTreeMap<String, PinnedMCPTool>> = call
        .pinned_tools
        .clone()
        .map(serde_json::from_value)
        .transpose()
        .or_value_error()?;
    let store = RowStore::new(&call.rows);
    let db_cipher = cipher(&call.salt_key);
    let pinned = set_mcp_server_pinned_tools(
        &store,
        &db_cipher,
        &call.server_id,
        tools.as_ref(),
        &call.touched_by,
    )
    .await
    .or_value_error()?;
    let Some(server) = pinned else {
        return Ok(Value::Null);
    };
    Ok(json!({"pinned_tools": serde_json::to_value(&server.pinned_tools).or_value_error()?}))
}

pub(super) async fn submissions(call: &Submissions) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let summary = get_mcp_submissions(&store, &cipher(&call.salt_key))
        .await
        .or_value_error()?;
    let mut server_ids = Vec::with_capacity(summary.items.len());
    for server in &summary.items {
        server_ids.push(server.server_id.as_str());
    }
    Ok(json!({
        "total": summary.total,
        "pending_review": summary.pending_review,
        "active": summary.active,
        "rejected": summary.rejected,
        "server_ids": server_ids,
    }))
}

pub(super) async fn active_submissions(call: &ActiveSubmissions) -> HelperResult {
    let store = RowStore::new(&call.rows);
    let ids = active_ids(&store, &call.user_id).await.or_value_error()?;
    Ok(Value::Array(ids.into_iter().map(Value::String).collect()))
}

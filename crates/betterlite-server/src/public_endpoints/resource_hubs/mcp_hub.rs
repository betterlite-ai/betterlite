use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTransport};
use betterlite_mcp::registry::public_servers::PublicServers as _;
use betterlite_mcp::registry::servers::ServerRegistry;
use betterlite_mcp::server_record::MCPServer;
use serde::Serialize;

use super::ResourceHubsState;
use super::error::ResourceHubsError;

#[derive(Debug, Serialize)]
pub struct PublicMcpServer {
    pub server_id: String,
    pub name: String,
    pub alias: Option<String>,
    pub server_name: Option<String>,
    pub transport: MCPTransport,
    pub spec_path: Option<String>,
    pub auth_type: Option<MCPAuth>,
    pub mcp_info: BTreeMap<String, JsonValue>,
}

pub async fn mcp_hub(
    State(state): State<ResourceHubsState>,
) -> Result<Json<Vec<PublicMcpServer>>, ResourceHubsError> {
    let registry = state.mcp.read().await;
    Ok(Json(public_mcp_servers(
        &registry,
        state.public_mcp_servers.as_deref(),
    )?))
}

#[must_use]
pub fn public_mcp_servers(
    registry: &ServerRegistry,
    explicit: Option<&[String]>,
) -> Result<Vec<PublicMcpServer>, ResourceHubsError> {
    let mut entries = Vec::new();
    for server in registry.public_servers() {
        entries.push(public_mcp_server(server, explicit)?);
    }
    Ok(entries)
}

fn public_mcp_server(
    server: &MCPServer,
    explicit: Option<&[String]>,
) -> Result<PublicMcpServer, ResourceHubsError> {
    let mut info = match server.fields.mcp_info.as_ref() {
        Some(stored) => stored_entries(stored)?,
        None => BTreeMap::new(),
    };
    info.insert(String::from("is_public"), JsonValue::Bool(true));
    let listed = explicit.is_some_and(|ids| ids.contains(&server.fields.server_id));
    info.insert(String::from("is_public_explicit"), JsonValue::Bool(listed));
    Ok(PublicMcpServer {
        server_id: server.fields.server_id.clone(),
        name: server.fields.name.clone(),
        alias: server.fields.alias.clone(),
        server_name: server.fields.server_name.clone(),
        transport: server.fields.transport,
        spec_path: server.fields.spec_path.clone(),
        auth_type: server.fields.auth_type,
        mcp_info: info,
    })
}

// MCPInfo exposes no entries, and the JSON object it serializes to is exactly what the response spreads.
fn stored_entries(info: &MCPInfo) -> Result<BTreeMap<String, JsonValue>, ResourceHubsError> {
    let encoded = serde_json::to_string(info).map_err(ResourceHubsError::McpInfo)?;
    serde_json::from_str(&encoded).map_err(ResourceHubsError::McpInfo)
}

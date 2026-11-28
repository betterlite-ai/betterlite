use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTransport};
use betterlite_models::decisions::Json;
use sha2::{Digest as _, Sha256};

use super::error::ConfigError;
use super::values::auth_text;

#[must_use]
pub fn generate_stable_server_id(
    server_name: &str,
    url: &str,
    transport: Option<MCPTransport>,
    auth_type: Option<MCPAuth>,
    alias: Option<&str>,
) -> String {
    let transport_text = match transport {
        None => "MCPTransport.http",
        Some(MCPTransport::Sse) => "sse",
        Some(MCPTransport::Http) => "http",
        Some(MCPTransport::Stdio) => "stdio",
    };
    let seed = format!(
        "{server_name}|{url}|{transport_text}|{}|{}",
        auth_type.map_or("", auth_text),
        alias.unwrap_or("")
    );
    let digest = Sha256::digest(seed.as_bytes());
    hex::encode(digest).chars().take(32).collect()
}

pub fn pinned_config_server_id(
    raw: Option<&Json>,
    server_name: &str,
) -> Result<Option<String>, ConfigError> {
    let Some(value) = raw else {
        return Ok(None);
    };
    let trimmed = match value {
        Json::Text(text) => text.trim(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => "",
    };
    if trimmed.is_empty() {
        return Err(ConfigError::ServerId {
            server_name: server_name.to_owned(),
        });
    }
    Ok(Some(trimmed.to_owned()))
}

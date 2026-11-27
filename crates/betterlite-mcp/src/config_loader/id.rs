use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTransport};
use betterlite_models::decisions::Json;
use sha2::{Digest as _, Sha256};

use super::error::ConfigError;
use super::values::{auth_text, python_repr, transport_seed};

#[must_use]
pub fn generate_stable_server_id(
    server_name: &str,
    url: &str,
    transport: Option<MCPTransport>,
    auth_type: Option<MCPAuth>,
    alias: Option<&str>,
) -> String {
    let seed = format!(
        "{server_name}|{url}|{}|{}|{}",
        transport_seed(transport),
        auth_type.map_or("", auth_text),
        alias.unwrap_or("")
    );
    let digest = Sha256::digest(seed.as_bytes());
    hex::encode(digest).chars().take(32).collect()
}

pub fn pinned_config_server_id(
    raw_server_id: Option<&Json>,
    server_name: &str,
) -> Result<Option<String>, ConfigError> {
    let Some(value) = raw_server_id else {
        return Ok(None);
    };
    let Json::Text(text) = value else {
        return Err(ConfigError::ServerId {
            server_name: server_name.to_owned(),
            raw: python_repr(value),
        });
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::ServerId {
            server_name: server_name.to_owned(),
            raw: python_repr(value),
        });
    }
    Ok(Some(trimmed.to_owned()))
}

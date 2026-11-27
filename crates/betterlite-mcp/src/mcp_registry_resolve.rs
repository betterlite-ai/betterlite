mod tests;

use crate::registry::servers::ServerRegistry;
use crate::server_record::MCPServer;

// LiteLLM's fallback when the registry has no entry for the requested server_id.
pub const FALLBACK_SERVER_NAME: &str = "MCP Server";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerMetadata {
    pub server_name: String,
    pub access_items: Vec<String>,
    pub help_url: String,
}

// LiteLLM reads server_name or name, byok_description and byok_api_key_help_url off the registry row.
#[must_use]
pub fn resolve(server_id: Option<&str>, registry: Option<&ServerRegistry>) -> ServerMetadata {
    let Some(server) = server_id.and_then(|server_id| find(server_id, registry)) else {
        return ServerMetadata {
            server_name: FALLBACK_SERVER_NAME.to_owned(),
            access_items: Vec::new(),
            help_url: String::new(),
        };
    };
    let fields = &server.fields;
    let server_name = fields
        .server_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(&fields.name);
    ServerMetadata {
        server_name: server_name.to_owned(),
        access_items: fields.byok_description.clone(),
        help_url: fields.byok_api_key_help_url.clone().unwrap_or_default(),
    }
}

#[must_use]
pub fn server_initial(server_name: &str) -> String {
    let Some(first) = server_name.chars().next() else {
        return "S".to_owned();
    };
    first.to_uppercase().collect()
}

fn find<'registry>(
    server_id: &str,
    registry: Option<&'registry ServerRegistry>,
) -> Option<&'registry MCPServer> {
    let registry = registry?;
    registry
        .get_registry()
        .into_iter()
        .find(|server| server.fields.server_id == server_id)
}

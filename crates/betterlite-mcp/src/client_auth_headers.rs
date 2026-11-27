pub mod lookup;
mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::server_record::MCPServer;
use lookup::ServerAuthHeaders;
use lookup::lookup_server_auth;

// ast-grep-ignore: no-long-names - LiteLLM's _client_has_per_server_auth_header
#[must_use]
pub fn client_has_per_server_auth_header(
    server: &MCPServer,
    server_auth_headers: Option<&ServerAuthHeaders>,
) -> bool {
    let Some(headers) = server_auth_headers else {
        return false;
    };
    let Some(resolved) = lookup_server_auth(
        headers,
        server.fields.alias.as_deref(),
        server.fields.server_name.as_deref(),
        server.fields.access_groups.as_deref().unwrap_or_default(),
    ) else {
        return false;
    };
    resolved
        .keys()
        .any(|name| name.eq_ignore_ascii_case("authorization"))
}

// ast-grep-ignore: no-long-names - LiteLLM's _client_has_passthrough_authorization
#[must_use]
pub fn client_has_passthrough_authorization(
    server: &MCPServer,
    oauth2_headers: Option<&BTreeMap<String, SecretString>>,
    server_auth_headers: Option<&ServerAuthHeaders>,
) -> bool {
    let top_level = oauth2_headers.is_some_and(|headers| {
        headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    });
    if top_level {
        return true;
    }
    // ast-grep-ignore: no-long-names - the per-server answer keeps LiteLLM's name
    client_has_per_server_auth_header(server, server_auth_headers)
}

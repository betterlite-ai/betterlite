mod tests;

use betterlite_mcp::registry::lookup::ServerLookups as _;
use betterlite_mcp::registry::servers::ServerRegistry;
use betterlite_mcp::request_headers::path_targets::resolve_target_server_names;
use betterlite_mcp::server_oauth::flow::effective_oauth2_flow;
use betterlite_mcp::server_record::{MCPServer, OAuth2Flow};

use betterlite_db::mcp_stored_contracts::MCPAuth;

// The route is the request path with the root path already stripped, and header_servers is the x-mcp-servers list when the request sent one.
#[derive(Clone, Copy)]
pub struct TargetQuery<'a> {
    pub registry: &'a ServerRegistry,
    pub route: &'a str,
    pub header_servers: Option<&'a [String]>,
    pub client_ip: Option<&'a str>,
}

pub struct BridgeTarget<'a> {
    pub requested_name: String,
    pub server: &'a MCPServer,
}

#[must_use]
pub fn is_true_passthrough(query: &TargetQuery<'_>) -> bool {
    let target_names = resolve_target_server_names(query.route, query.header_servers);
    if target_names.is_empty() {
        return false;
    }
    target_names.iter().all(|name| {
        query
            .registry
            .server_by_name(name, query.client_ip)
            .is_some_and(MCPServer::is_true_passthrough)
    })
}

#[must_use]
pub fn single_bridge_target<'a>(query: &TargetQuery<'a>) -> Option<BridgeTarget<'a>> {
    let target_names = resolve_target_server_names(query.route, query.header_servers);
    let [requested_name] = target_names.as_slice() else {
        return None;
    };
    let server = query
        .registry
        .server_by_name(requested_name, query.client_ip)?;
    let forwards_client_token = server.is_oauth_delegate() && server.is_dcr_bridge();
    // Egress finds the injected per-server token only by alias or server_name, so a server with neither cannot receive it.
    let has_egress_name = server
        .fields
        .server_name
        .as_deref()
        .is_some_and(|name| !name.is_empty())
        || server
            .fields
            .alias
            .as_deref()
            .is_some_and(|alias| !alias.is_empty());
    (forwards_client_token && has_egress_name).then(|| BridgeTarget {
        requested_name: requested_name.clone(),
        server,
    })
}

#[must_use]
pub fn is_passthrough_cold_start(
    registry: &ServerRegistry,
    names: &[String],
    client_ip: Option<&str>,
) -> bool {
    !names.is_empty()
        && names.iter().all(|name| {
            registry
                .server_by_name(name, client_ip)
                .is_some_and(MCPServer::is_oauth_passthrough)
        })
}

#[must_use]
pub fn is_legacy_delegate_cold_start(
    registry: &ServerRegistry,
    names: &[String],
    client_ip: Option<&str>,
) -> bool {
    if names.is_empty() {
        return false;
    }
    for name in names {
        let Some(server) = registry.server_by_name(name, client_ip) else {
            return false;
        };
        let is_oauth2_delegate = server.fields.auth_type == Some(MCPAuth::Oauth2)
            && server.fields.delegate_auth_to_upstream;
        if !is_oauth2_delegate {
            return false;
        }
        if effective_oauth2_flow(server) == Some(OAuth2Flow::ClientCredentials) {
            return false;
        }
    }
    true
}

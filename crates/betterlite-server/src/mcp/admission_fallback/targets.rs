mod tests;

use betterlite_mcp::registry::lookup::ServerLookups as _;
use betterlite_mcp::registry::servers::ServerRegistry;
use betterlite_mcp::request_headers::path_targets::resolve_target_server_names;
use betterlite_mcp::server_oauth::flow::effective_oauth2_flow;
use betterlite_mcp::server_record::{MCPServer, OAuth2Flow};

use betterlite_db::mcp_stored_contracts::MCPAuth;

// The route is the request path with the root path already stripped, and header_servers is the x-mcp-servers list when the request sent one.
#[derive(Clone, Copy)]
pub struct TargetQuery<'request> {
    pub registry: &'request ServerRegistry,
    pub route: &'request str,
    pub header_servers: Option<&'request [String]>,
    pub client_ip: Option<&'request str>,
}

pub struct BridgeTarget<'request> {
    pub requested_name: String,
    pub server: &'request MCPServer,
}

#[must_use]
pub fn is_true_passthrough(query: &TargetQuery<'_>) -> bool {
    let target_names = resolve_target_server_names(query.route, query.header_servers);
    if target_names.is_empty() {
        return false;
    }
    every_resolved_server(query, &target_names, MCPServer::is_true_passthrough)
}

#[must_use]
pub fn single_bridge_target<'request>(
    query: &TargetQuery<'request>,
) -> Option<BridgeTarget<'request>> {
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
pub fn is_passthrough_cold_start(query: &TargetQuery<'_>, names: &[String]) -> bool {
    if names.is_empty() {
        return false;
    }
    every_resolved_server(query, names, MCPServer::is_oauth_passthrough)
}

#[must_use]
pub fn is_legacy_delegate_cold_start(query: &TargetQuery<'_>, names: &[String]) -> bool {
    if names.is_empty() {
        return false;
    }
    every_resolved_server(query, names, is_legacy_delegate_server)
}

// Callers check the list is non-empty; an empty list would make the all() vacuously true.
fn every_resolved_server(
    query: &TargetQuery<'_>,
    names: &[String],
    predicate: fn(&MCPServer) -> bool,
) -> bool {
    names.iter().all(|name| {
        query
            .registry
            .server_by_name(name, query.client_ip)
            .is_some_and(predicate)
    })
}

fn is_legacy_delegate_server(server: &MCPServer) -> bool {
    server.fields.auth_type == Some(MCPAuth::Oauth2)
        && server.fields.delegate_auth_to_upstream
        && effective_oauth2_flow(server) != Some(OAuth2Flow::ClientCredentials)
}

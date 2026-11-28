mod tests;

use crate::gateway_dcr::models::ConnectFlow;
use crate::gateway_dcr::ports::LookupServerReachability;
use crate::oauth_resource::resource_identity::{canonical_resource_uri, canonicalize_url_identity};
use crate::registry::lookup::ServerLookups as _;
use crate::registry::servers::ServerRegistry;
use crate::request_headers::path_targets::target_names_from_path;
use crate::server_oauth::flow::effective_oauth2_flow;
use crate::server_record::{MCPServer, OAuth2Flow};

#[must_use]
pub fn is_proxy_api_resource(base_url: &str, resource: Option<&str>) -> bool {
    let Some(present) = resource else {
        return false;
    };
    let Ok(Some(canonical_uri)) = canonical_resource_uri(present) else {
        return false;
    };
    canonicalize_url_identity(base_url).is_ok_and(|base| canonical_uri == base)
}

#[must_use]
pub fn resolve_scoped_resource_server<'registry>(
    registry: &'registry ServerRegistry,
    base_url: &str,
    resource: Option<&str>,
) -> Option<&'registry MCPServer> {
    let canonical = canonical_resource_uri(resource?).ok()??;
    let base = canonicalize_url_identity(base_url).ok()?;
    let tail = canonical.strip_prefix(base.as_str())?;
    if tail == "/mcp" || !tail.starts_with('/') {
        return None;
    }
    let names = target_names_from_path(tail);
    let [name] = names.as_slice() else {
        return None;
    };
    let server = registry.server_by_name(name, None)?;
    (server.is_gateway_managed_oauth2() || server.advertises_gateway_authorization_server())
        .then_some(server)
}

#[derive(Debug)]
pub enum FlowTarget<'server> {
    Unscoped,
    Interactive(&'server MCPServer),
    M2m(&'server MCPServer),
    Stale,
}

#[must_use]
pub async fn flow_target<'registry, R>(
    registry: &'registry ServerRegistry,
    flow: &ConnectFlow,
    reachability: &R,
) -> FlowTarget<'registry>
where
    R: LookupServerReachability + ?Sized,
{
    let Some(server_id) = flow.resource_server_id.as_deref() else {
        return FlowTarget::Unscoped;
    };
    let Some(server) = registry.server_by_id(server_id, None) else {
        return FlowTarget::Stale;
    };
    if !server.is_gateway_managed_oauth2() && !server.advertises_gateway_authorization_server() {
        return FlowTarget::Stale;
    }
    if !reachability
        .reachable(&flow.user_id, &server.fields.server_id)
        .await
    {
        return FlowTarget::Stale;
    }
    if server.is_gateway_managed_oauth2()
        && effective_oauth2_flow(server) != Some(OAuth2Flow::ClientCredentials)
    {
        FlowTarget::Interactive(server)
    } else {
        FlowTarget::M2m(server)
    }
}

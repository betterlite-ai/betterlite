use betterlite_db::managed_resource_rows::mcp::McpServerTable;

use super::OAuthResolution;
use super::TableOAuthRequest;
use super::request::{flow_name, manual_endpoints};
use crate::server_build::credentials::ResolvedCredentials;
use crate::server_oauth::flow::explicit_oauth2_flow;
use crate::server_record::MCPOAuthMetadata;

struct DiscoveredOAuth {
    scopes: Option<Vec<String>>,
    issuer: Option<String>,
    authorization_url: Option<String>,
    token_url: Option<String>,
    registration_url: Option<String>,
}

fn discovered_oauth(metadata: Option<&MCPOAuthMetadata>) -> DiscoveredOAuth {
    DiscoveredOAuth {
        scopes: metadata.and_then(|found| found.scopes.clone()),
        issuer: metadata
            .filter(|found| !found.from_origin_fallback)
            .and_then(|found| found.discovered_issuer.clone()),
        authorization_url: metadata.and_then(|found| found.authorization_url.clone()),
        token_url: metadata.and_then(|found| found.token_url.clone()),
        registration_url: metadata.and_then(|found| found.registration_url.clone()),
    }
}

pub(super) fn resolve_oauth_fields(
    row: &McpServerTable,
    credentials: &ResolvedCredentials,
    request: &TableOAuthRequest<'_>,
    metadata: Option<MCPOAuthMetadata>,
) -> OAuthResolution {
    let discovered = discovered_oauth(metadata.as_ref());
    let configured = manual_endpoints(row);
    OAuthResolution {
        oauth2_flow: explicit_oauth2_flow(row.oauth2_flow.map(flow_name)),
        scopes: credentials.scopes.clone().or(discovered.scopes),
        configured_scopes: credentials.configured_scopes.clone(),
        issuer: request
            .manual_issuer
            .map(str::to_owned)
            .or(discovered.issuer),
        issuer_is_anchored: request.use_issuer_anchor,
        authorization_url: request
            .manual_authorization_url
            .map(str::to_owned)
            .or(discovered.authorization_url),
        token_url: request
            .manual_token_url
            .map(str::to_owned)
            .or(discovered.token_url),
        registration_url: request
            .manual_registration_url
            .map(str::to_owned)
            .or(discovered.registration_url),
        configured_authorization_url: configured.authorization_url.map(str::to_owned),
        configured_token_url: configured.token_url.map(str::to_owned),
        configured_registration_url: configured.registration_url.map(str::to_owned),
        token_endpoint_auth_method: credentials.token_endpoint_auth_method,
    }
}

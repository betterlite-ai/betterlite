mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_mcp::oauth_resource::base_url::{RequestOrigin, get_request_base_url};
use betterlite_mcp::oauth_resource::challenge::{
    passthrough_metadata_url, passthrough_www_authenticate, route_relative_request_path,
    well_known_root_suffix,
};
use betterlite_mcp::registry::lookup::ServerLookups as _;
use betterlite_mcp::request_headers::path_targets::{
    cold_start_server_names, target_names_from_path,
};
use serde::Serialize;
use tracing::warn;

use super::targets::TargetQuery;
use crate::mcp::transport_checks::McpHttpError;

const UNAUTHORIZED_STATUS: u16 = 401;
const INVALID_CREDENTIAL_DETAIL: &str = "Invalid or expired credential";

pub type ChallengeResponse = McpHttpError;

// What the URL builders need from the request; RequestOrigin carries the trusted-proxy decision.
#[derive(Clone, Copy)]
pub struct ChallengeContext<'a> {
    pub proxy_base_url: Option<&'a str>,
    pub origin: RequestOrigin<'a>,
    pub server_root_path: &'a str,
    pub original_path: Option<&'a str>,
    pub path: &'a str,
    pub app_root_path: Option<&'a str>,
    pub root_path: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub struct McpAuthHeaders<'a> {
    pub mcp_auth_header: Option<&'a str>,
    pub server_headers: &'a BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Serialize)]
struct GatewayChallengeDetail {
    error: &'static str,
    message: &'static str,
}

#[must_use]
pub fn has_client_supplied_mcp_auth(
    mcp_auth_header: Option<&str>,
    server_headers: &BTreeMap<String, BTreeMap<String, String>>,
) -> bool {
    let has_header = mcp_auth_header.is_some_and(|value| !value.is_empty());
    let has_server_headers = !server_headers.is_empty();
    has_header || has_server_headers
}

#[must_use]
pub fn challenge_target(query: &TargetQuery<'_>) -> Option<String> {
    let target = cold_start_server_names(query.route, query.header_servers)?;
    let server = query.registry.server_by_name(&target, query.client_ip)?;
    server
        .advertises_gateway_authorization_server()
        .then_some(target)
}

#[must_use]
pub fn is_dcr_challenge_scope(
    query: &TargetQuery<'_>,
    auth: &McpAuthHeaders<'_>,
    error_is_401: bool,
) -> bool {
    if !error_is_401 {
        return false;
    }
    if has_client_supplied_mcp_auth(auth.mcp_auth_header, auth.server_headers) {
        return false;
    }
    if target_names_from_path(query.route).is_empty() {
        return true;
    }
    challenge_target(query).is_some()
}

#[must_use]
pub fn gateway_dcr_challenge(
    ctx: &ChallengeContext<'_>,
    query: &TargetQuery<'_>,
    invalid_token: bool,
) -> ChallengeResponse {
    let resource_metadata_url = match challenge_target(query) {
        Some(target) => per_server_metadata_url(ctx, &target),
        None => aggregate_metadata_url(ctx),
    };
    ChallengeResponse {
        status: UNAUTHORIZED_STATUS,
        detail: gateway_detail(),
        headers: vec![(
            "WWW-Authenticate".to_owned(),
            passthrough_www_authenticate(&resource_metadata_url, invalid_token),
        )],
    }
}

#[must_use]
pub fn bridge_invalid_token_challenge(
    ctx: &ChallengeContext<'_>,
    requested_name: &str,
) -> ChallengeResponse {
    let resource_metadata_url = per_server_metadata_url(ctx, requested_name);
    ChallengeResponse {
        status: UNAUTHORIZED_STATUS,
        detail: JsonValue::String(INVALID_CREDENTIAL_DETAIL.to_owned()),
        headers: vec![(
            "www-authenticate".to_owned(),
            passthrough_www_authenticate(&resource_metadata_url, true),
        )],
    }
}

fn gateway_detail() -> JsonValue {
    let detail = GatewayChallengeDetail {
        error: "authentication_required",
        message: "Authenticate with the gateway to use the MCP endpoint.",
    };
    serde_json::to_value(detail).unwrap_or(JsonValue::Null)
}

fn request_base_url(ctx: &ChallengeContext<'_>) -> String {
    let proxy_base_url = ctx.proxy_base_url.unwrap_or_default();
    match get_request_base_url(proxy_base_url, &ctx.origin) {
        Ok(base_url) => base_url,
        Err(error) => {
            warn!(
                error = %error,
                "MCP challenge: the request base URL could not be derived, so the client's own base URL is advertised"
            );
            ctx.origin.base_url.trim_end_matches('/').to_owned()
        }
    }
}

fn aggregate_metadata_url(ctx: &ChallengeContext<'_>) -> String {
    let base_url = request_base_url(ctx);
    let suffix = well_known_root_suffix(ctx.server_root_path);
    format!("{base_url}/.well-known/oauth-protected-resource{suffix}/mcp")
}

fn per_server_metadata_url(ctx: &ChallengeContext<'_>, server_name: &str) -> String {
    let base_url = request_base_url(ctx);
    let route_path = route_relative_request_path(
        ctx.original_path,
        ctx.path,
        ctx.app_root_path,
        ctx.root_path,
    );
    let suffix = well_known_root_suffix(ctx.server_root_path);
    passthrough_metadata_url(&base_url, &route_path, server_name, suffix)
}

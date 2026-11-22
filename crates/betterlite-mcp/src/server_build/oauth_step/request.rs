use betterlite_db::managed_resource_rows::mcp::{McpOAuth2Flow, McpServerTable};
use betterlite_db::mcp_stored_contracts::MCPAuth;

use super::TableOAuthRequest;
use crate::server_build::credentials::ResolvedCredentials;
use crate::server_build::fields::first_non_empty;
use crate::server_oauth::basics::{
    UPSTREAM_DISCOVERY_AUTH_TYPES, blank_to_none, obo_needs_discovery, uses_issuer_anchor,
};
use crate::server_oauth::issuer::{ManualEndpoints, endpoints_yield_to_issuer};
use crate::server_record::ServerFlag;

struct TableOAuthFlags {
    auth_type: Option<MCPAuth>,
    is_discovery_auth_type: ServerFlag,
    use_issuer_anchor: ServerFlag,
    token_exchange_endpoint: Option<String>,
}

fn table_oauth_flags(row: &McpServerTable, credentials: &ResolvedCredentials) -> TableOAuthFlags {
    let auth_type = row.auth_type;
    let is_discovery_auth_type =
        auth_type.is_some_and(|auth| UPSTREAM_DISCOVERY_AUTH_TYPES.contains(&auth));
    let manual_token_url = blank_to_none(row.token_url.as_deref());
    let token_exchange_endpoint = first_non_empty(
        row.token_exchange_endpoint.as_deref(),
        credentials.token_exchange_endpoint.as_deref(),
    );
    let obo = obo_needs_discovery(
        auth_type,
        token_exchange_endpoint.as_deref(),
        manual_token_url,
    );
    TableOAuthFlags {
        auth_type,
        is_discovery_auth_type,
        use_issuer_anchor: uses_issuer_anchor(
            blank_to_none(row.issuer.as_deref()),
            is_discovery_auth_type || obo,
        ),
        token_exchange_endpoint,
    }
}

pub(super) fn table_oauth_request<'table>(
    row: &'table McpServerTable,
    credentials: &'table ResolvedCredentials,
    name: &'table str,
) -> TableOAuthRequest<'table> {
    let manual_issuer = blank_to_none(row.issuer.as_deref());
    let flags = table_oauth_flags(row, credentials);
    let yielded = endpoints_yield_to_issuer(
        manual_issuer,
        flags.is_discovery_auth_type,
        manual_endpoints(row),
        name,
    );
    TableOAuthRequest {
        server_id: &row.server_id,
        server_ref: name,
        server_url: row.url.as_deref(),
        auth_type: flags.auth_type,
        oauth2_flow: row.oauth2_flow,
        dcr_bridge: row.dcr_bridge,
        manual_issuer,
        manual_authorization_url: yielded.authorization_url,
        manual_token_url: yielded.token_url,
        manual_registration_url: yielded.registration_url,
        is_discovery_auth_type: flags.is_discovery_auth_type,
        use_issuer_anchor: flags.use_issuer_anchor,
        scopes: credentials.scopes.as_deref(),
        token_exchange_endpoint: flags.token_exchange_endpoint,
    }
}

pub(super) fn manual_endpoints(row: &McpServerTable) -> ManualEndpoints<'_> {
    ManualEndpoints {
        authorization_url: blank_to_none(row.authorization_url.as_deref()),
        token_url: blank_to_none(row.token_url.as_deref()),
        registration_url: blank_to_none(row.registration_url.as_deref()),
    }
}

pub(super) const fn flow_name(flow: McpOAuth2Flow) -> &'static str {
    match flow {
        McpOAuth2Flow::ClientCredentials => "client_credentials",
        McpOAuth2Flow::AuthorizationCode => "authorization_code",
    }
}

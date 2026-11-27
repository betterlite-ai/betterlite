pub mod errors;
mod tests;

use betterlite_db::mcp_stored_contracts::MCPAuth;
use betterlite_mcp::gateway_dcr::contracts::supported_grant_types;
use betterlite_mcp::server_record::MCPServer;
use betterlite_models::decisions::Json;
use serde::Serialize;

use self::errors::{DiscoveryFailure, not_oauth_protected};
use super::upstream_fetch::UpstreamMetadataFetch as _;
use super::{GatewayFacts, JsonObject, ProtectedRoot, RequestFacts, UpstreamGet};

#[derive(Debug, Serialize)]
pub struct RootResource {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GatewayResource {
    pub authorization_servers: Vec<String>,
    pub resource: String,
    pub scopes_supported: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AggregateAuthServer {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub introspection_endpoint: String,
    pub registration_endpoint: String,
    pub response_types_supported: Vec<&'static str>,
    pub scopes_supported: Vec<&'static str>,
    pub grant_types_supported: Vec<&'static str>,
    #[serde(rename = "code_challenge_methods_supported")]
    pub code_challenge_methods: Vec<&'static str>,
    #[serde(rename = "token_endpoint_auth_methods_supported")]
    pub token_auth_methods: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum NamedDocument {
    Gateway(GatewayResource),
    Upstream(JsonObject),
}

#[must_use]
pub fn root_resource(base_url: &str) -> RootResource {
    RootResource {
        resource: origin(base_url),
        authorization_servers: vec![format!("{base_url}/mcp")],
        scopes_supported: Vec::new(),
    }
}

#[must_use]
pub fn aggregate_protected_resource(base_url: &str) -> GatewayResource {
    GatewayResource {
        authorization_servers: vec![format!("{base_url}/mcp")],
        resource: format!("{base_url}/mcp"),
        scopes_supported: Vec::new(),
    }
}

#[must_use]
pub fn aggregate_auth_server(
    base_url: &str,
    token_exchange_available: bool,
) -> AggregateAuthServer {
    AggregateAuthServer {
        issuer: format!("{base_url}/mcp"),
        authorization_endpoint: format!("{base_url}/authorize/mcp-session"),
        token_endpoint: format!("{base_url}/token"),
        introspection_endpoint: format!("{base_url}/introspect"),
        registration_endpoint: format!("{base_url}/register"),
        response_types_supported: vec!["code"],
        scopes_supported: Vec::new(),
        grant_types_supported: supported_grant_types(token_exchange_available),
        code_challenge_methods: vec!["S256"],
        token_auth_methods: vec!["none", "client_secret_post"],
    }
}

#[must_use]
pub fn merge_issuers(env_issuer: Option<&str>, configured: &[Option<String>]) -> Vec<String> {
    let mut issuers: Vec<String> = Vec::new();
    if let Some(issuer) = env_issuer.filter(|issuer| !issuer.is_empty()) {
        issuers.push(issuer.to_owned());
    }
    for configured_issuer in configured.iter().flatten() {
        if configured_issuer.is_empty() || issuers.contains(configured_issuer) {
            continue;
        }
        issuers.push(configured_issuer.clone());
    }
    issuers
}

pub(crate) async fn named_resource<U, R, G>(
    root: &ProtectedRoot<U, R, G>,
    base_url: &str,
    name: &str,
    standard: bool,
    server: Option<&MCPServer>,
) -> Result<NamedDocument, DiscoveryFailure>
where
    U: UpstreamGet,
    R: RequestFacts,
    G: GatewayFacts,
{
    let resource_url = if standard {
        format!("{base_url}/mcp/{name}")
    } else {
        format!("{base_url}/{name}/mcp")
    };
    let Some(resolved) = server else {
        return Err(not_oauth_protected(name));
    };
    if resolved.is_dcr_bridge() {
        return Ok(NamedDocument::Gateway(GatewayResource {
            authorization_servers: vec![format!("{base_url}/{name}")],
            resource: resource_url,
            scopes_supported: resolved.fields.scopes.clone().unwrap_or_default(),
        }));
    }
    if resolved.is_oauth_passthrough()
        || resolved.is_oauth_delegate()
        || resolved.is_true_passthrough()
    {
        return pass_through_document(root, resolved, &resource_url).await;
    }
    gateway_or_refusal(root, base_url, name, &resource_url, resolved)
}

fn gateway_or_refusal<U, R, G>(
    root: &ProtectedRoot<U, R, G>,
    base_url: &str,
    name: &str,
    resource_url: &str,
    server: &MCPServer,
) -> Result<NamedDocument, DiscoveryFailure>
where
    G: GatewayFacts,
{
    if server.fields.auth_type == Some(MCPAuth::Oauth2TokenExchange) {
        let issuers = root.gateway.jwt_issuers();
        if !issuers.is_empty() {
            return Ok(NamedDocument::Gateway(GatewayResource {
                authorization_servers: issuers,
                resource: resource_url.to_owned(),
                scopes_supported: server.fields.scopes.clone().unwrap_or_default(),
            }));
        }
    }
    if server.advertises_gateway_authorization_server() {
        return Ok(NamedDocument::Gateway(GatewayResource {
            authorization_servers: vec![format!("{base_url}/mcp")],
            resource: resource_url.to_owned(),
            scopes_supported: server.fields.scopes.clone().unwrap_or_default(),
        }));
    }
    if !matches!(
        server.fields.auth_type,
        Some(MCPAuth::Oauth2TokenExchange | MCPAuth::Oauth2)
    ) {
        return Err(not_oauth_protected(name));
    }
    Ok(NamedDocument::Gateway(GatewayResource {
        authorization_servers: vec![format!("{base_url}/{name}")],
        resource: resource_url.to_owned(),
        scopes_supported: server.fields.scopes.clone().unwrap_or_default(),
    }))
}

async fn pass_through_document<U, R, G>(
    root: &ProtectedRoot<U, R, G>,
    server: &MCPServer,
    resource_url: &str,
) -> Result<NamedDocument, DiscoveryFailure>
where
    U: UpstreamGet,
    R: RequestFacts,
    G: GatewayFacts,
{
    match root.fetch_upstream_oauth_protected_resource(server).await {
        Ok(Some(document)) => {
            if server.is_client_forwarded_token() {
                return Ok(NamedDocument::Upstream(document));
            }
            Ok(NamedDocument::Upstream(rewritten_resource(
                document,
                resource_url,
            )))
        }
        Ok(None) => {
            tracing::warn!(
                mcp_server = %server.fields.name,
                "upstream oauth-protected-resource metadata is unavailable for a pass-through MCP server"
            );
            Err(errors::upstream_failure(
                &server.fields.name,
                "Upstream oauth-protected-resource metadata unavailable",
            ))
        }
        Err(cause) => {
            tracing::warn!(
                mcp_server = %server.fields.name,
                error = %cause,
                "fetching upstream oauth-protected-resource metadata failed for a pass-through MCP server"
            );
            Err(errors::upstream_failure(
                &server.fields.name,
                "Failed to fetch upstream oauth-protected-resource metadata",
            ))
        }
    }
}

fn origin(base_url: &str) -> String {
    match base_url.split_once("://") {
        Some((scheme, rest)) => {
            let netloc_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let netloc = rest.get(..netloc_end).unwrap_or_default();
            let lowered = scheme.to_ascii_lowercase();
            format!("{lowered}://{netloc}")
        }
        None => base_url.to_owned(),
    }
}

fn rewritten_resource(mut document: JsonObject, resource_url: &str) -> JsonObject {
    document.insert("resource".to_owned(), Json::Text(resource_url.to_owned()));
    document
}

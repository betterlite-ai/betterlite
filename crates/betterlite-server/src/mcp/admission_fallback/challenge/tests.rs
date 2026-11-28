#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_mcp::oauth_resource::base_url::RequestOrigin;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use serde_json::json;

use super::{
    ChallengeContext, GatewayChallengeDetail, McpAuthHeaders, bridge_invalid_token_challenge,
    challenge_target, gateway_dcr_challenge, has_client_supplied_mcp_auth, is_dcr_challenge_scope,
};
use crate::mcp::admission_fallback::targets::TargetQuery;

const GATEWAY: &str =
    r#"{"server_id":"srv-gateway","name":"gateway","transport":"http","auth_type":"oauth2"}"#;
const PRIVATE: &str = r#"{"server_id":"srv-private","name":"private","transport":"http","auth_type":"bearer_token","extra_headers":["authorization"]}"#;
const PASSTHROUGH: &str =
    r#"{"server_id":"srv-pt","name":"pt","transport":"http","auth_type":"true_passthrough"}"#;

fn server(source: &str) -> MCPServer {
    serde_json::from_str(source).unwrap()
}

fn registry(records: &[&str]) -> ServerRegistry {
    let servers: Vec<MCPServer> = records.iter().copied().map(server).collect();
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: true,
            internal_ip_ranges: None,
        },
        Vec::new(),
        servers,
    )
}

fn query<'request>(
    registry: &'request ServerRegistry,
    route: &'request str,
) -> TargetQuery<'request> {
    TargetQuery {
        registry,
        route,
        header_servers: None,
        client_ip: None,
    }
}

fn context<'request>(base_url: &'request str, path: &'request str) -> ChallengeContext<'request> {
    ChallengeContext {
        proxy_base_url: None,
        origin: RequestOrigin {
            base_url,
            x_forwarded_proto: None,
            x_forwarded_host: None,
            x_forwarded_port: None,
            from_trusted_proxy: false,
        },
        server_root_path: "/",
        original_path: None,
        path,
        app_root_path: None,
        root_path: None,
    }
}

fn authenticate<'request>(
    mcp_auth_header: Option<&'request str>,
    server_headers: &'request BTreeMap<String, BTreeMap<String, String>>,
) -> McpAuthHeaders<'request> {
    McpAuthHeaders {
        mcp_auth_header,
        server_headers,
    }
}

#[test]
fn client_auth_from_the_header_text() {
    let headers = BTreeMap::new();
    assert!(has_client_supplied_mcp_auth(Some("Bearer abc"), &headers));
    assert!(!has_client_supplied_mcp_auth(Some(""), &headers));
    assert!(!has_client_supplied_mcp_auth(None, &headers));
}

#[test]
fn client_auth_from_an_empty_alias_map() {
    let mut headers = BTreeMap::new();
    headers.insert("gateway".to_owned(), BTreeMap::new());
    assert!(has_client_supplied_mcp_auth(None, &headers));
}

#[test]
fn challenge_target_needs_gateway_sign_in() {
    let registry = registry(&[GATEWAY, PRIVATE, PASSTHROUGH]);
    assert_eq!(
        challenge_target(&query(&registry, "/mcp/gateway")),
        Some("gateway".to_owned())
    );
    assert!(challenge_target(&query(&registry, "/mcp/private")).is_none());
    assert!(challenge_target(&query(&registry, "/mcp/pt")).is_none());
    assert!(challenge_target(&query(&registry, "/mcp/missing")).is_none());
    assert!(challenge_target(&query(&registry, "/mcp")).is_none());
}

#[test]
fn challenge_scope_covers_the_aggregate_endpoint() {
    let registry = registry(&[]);
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    assert!(is_dcr_challenge_scope(
        &query(&registry, "/mcp"),
        &auth,
        true
    ));
    assert!(!is_dcr_challenge_scope(
        &query(&registry, "/mcp"),
        &auth,
        false
    ));
}

#[test]
fn challenge_scope_refuses_client_supplied_auth() {
    let registry = registry(&[]);
    let headers = BTreeMap::new();
    let with_header = authenticate(Some("Bearer abc"), &headers);
    let mut alias_map = BTreeMap::new();
    alias_map.insert("gateway".to_owned(), BTreeMap::new());
    let with_map = authenticate(None, &alias_map);
    assert!(!is_dcr_challenge_scope(
        &query(&registry, "/mcp"),
        &with_header,
        true
    ));
    assert!(!is_dcr_challenge_scope(
        &query(&registry, "/mcp"),
        &with_map,
        true
    ));
}

#[test]
fn challenge_scope_follows_the_named_target() {
    let registry = registry(&[GATEWAY, PRIVATE]);
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    assert!(is_dcr_challenge_scope(
        &query(&registry, "/mcp/gateway"),
        &auth,
        true
    ));
    assert!(!is_dcr_challenge_scope(
        &query(&registry, "/mcp/private"),
        &auth,
        true
    ));
    assert!(!is_dcr_challenge_scope(
        &query(&registry, "/mcp/missing"),
        &auth,
        true
    ));
}

#[test]
fn gateway_challenge_aggregate_url() {
    let registry = registry(&[]);
    let ctx = context("http://testserver/", "/mcp");
    let response = gateway_dcr_challenge(&ctx, &query(&registry, "/mcp"), false);
    let url = "http://testserver/.well-known/oauth-protected-resource/mcp";
    assert_eq!(response.status, 401);
    assert_eq!(
        response.detail,
        serde_json::to_value(GatewayChallengeDetail {
            error: "authentication_required",
            message: "Authenticate with the gateway to use the MCP endpoint.",
        })
        .unwrap()
    );
    assert_eq!(
        response.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn gateway_challenge_invalid_token_form() {
    let registry = registry(&[]);
    let ctx = context("http://testserver/", "/mcp");
    let response = gateway_dcr_challenge(&ctx, &query(&registry, "/mcp"), true);
    let url = "http://testserver/.well-known/oauth-protected-resource/mcp";
    assert_eq!(
        response.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer error=\"invalid_token\", resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn gateway_challenge_per_server_url() {
    let registry = registry(&[GATEWAY]);
    let ctx = context("http://testserver/", "/mcp/gateway");
    let response = gateway_dcr_challenge(&ctx, &query(&registry, "/mcp/gateway"), false);
    let url = "http://testserver/.well-known/oauth-protected-resource/mcp/gateway";
    assert_eq!(
        response.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn gateway_challenge_keeps_the_server_first_spelling() {
    let registry = registry(&[GATEWAY]);
    let ctx = context("http://testserver/", "/gateway/mcp");
    let response = gateway_dcr_challenge(&ctx, &query(&registry, "/gateway/mcp"), false);
    let url = "http://testserver/.well-known/oauth-protected-resource/gateway/mcp";
    assert_eq!(
        response.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn gateway_challenge_inserts_the_root_path_suffix() {
    let registry = registry(&[]);
    let mut ctx = context("http://testserver/litellm/", "/litellm/mcp");
    ctx.server_root_path = "/litellm";
    ctx.root_path = Some("/litellm");
    let response = gateway_dcr_challenge(&ctx, &query(&registry, "/mcp"), false);
    let url = "http://testserver/litellm/.well-known/oauth-protected-resource/litellm/mcp";
    assert_eq!(
        response.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn gateway_challenge_prefers_the_configured_proxy_base_url() {
    let registry = registry(&[]);
    let mut ctx = context("http://testserver/", "/mcp");
    ctx.proxy_base_url = Some("https://proxy.example/");
    let response = gateway_dcr_challenge(&ctx, &query(&registry, "/mcp"), false);
    let url = "https://proxy.example/.well-known/oauth-protected-resource/mcp";
    assert_eq!(
        response.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn gateway_challenge_survives_an_unparsable_base_url() {
    let registry = registry(&[]);
    let ctx = context("http://[::1/", "/mcp");
    let response = gateway_dcr_challenge(&ctx, &query(&registry, "/mcp"), false);
    let url = "http://[::1/.well-known/oauth-protected-resource/mcp";
    assert_eq!(response.status, 401);
    assert_eq!(
        response.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn gateway_challenge_reads_a_forwarded_host_from_a_trusted_proxy() {
    let registry = registry(&[]);
    let mut ctx = context("http://internal/", "/mcp");
    ctx.origin.from_trusted_proxy = true;
    ctx.origin.x_forwarded_host = Some("public.example");
    let response = gateway_dcr_challenge(&ctx, &query(&registry, "/mcp"), false);
    let url = "http://public.example/.well-known/oauth-protected-resource/mcp";
    assert_eq!(
        response.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn bridge_challenge_spells_its_header_in_lowercase() {
    let ctx = context("http://testserver/", "/gateway/mcp");
    let response = bridge_invalid_token_challenge(&ctx, "gateway");
    let url = "http://testserver/.well-known/oauth-protected-resource/gateway/mcp";
    assert_eq!(response.status, 401);
    assert_eq!(response.detail, json!("Invalid or expired credential"));
    assert_eq!(
        response.headers,
        vec![(
            "www-authenticate".to_owned(),
            format!("Bearer error=\"invalid_token\", resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn bridge_challenge_keeps_the_alias_spelling() {
    let ctx = context("http://testserver/", "/mcp/bridge-alias");
    let response = bridge_invalid_token_challenge(&ctx, "bridge-alias");
    let url = "http://testserver/.well-known/oauth-protected-resource/mcp/bridge-alias";
    assert_eq!(
        response.headers,
        vec![(
            "www-authenticate".to_owned(),
            format!("Bearer error=\"invalid_token\", resource_metadata=\"{url}\""),
        )]
    );
}

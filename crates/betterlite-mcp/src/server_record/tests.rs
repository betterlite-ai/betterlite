#![cfg(test)]

use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTransport, MCPUpstreamProtocol};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::binding::{BindingMode, CallerField};
use super::{MCPOAuthIdentityBinding, MCPOAuthMetadata, MCPServer, OAuth2Flow};

mod fixtures;

use crate::credential_contracts::auth_config::oauth::DEFAULT_SUBJECT_TOKEN_TYPE;
use fixtures::{FULL_SERVER_JSON, SECRET_SERVER_JSON};

fn server_from_json(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

fn base_server() -> MCPServer {
    server_from_json(r#"{"server_id": "srv", "name": "Server", "transport": "http"}"#)
}

fn with_headers(mut server: MCPServer, headers: &[&str]) -> MCPServer {
    server.extra_headers =
        (!headers.is_empty()).then(|| headers.iter().map(|header| (*header).to_owned()).collect());
    server
}

#[rstest]
#[case::missing_server_id(r#"{"name": "Server", "transport": "http"}"#, "server_id")]
#[case::missing_name(r#"{"server_id": "srv", "transport": "http"}"#, "name")]
#[case::missing_transport(r#"{"server_id": "srv", "name": "Server"}"#, "transport")]
fn record_requires_server_id_name_and_transport(#[case] json: &str, #[case] field: &str) {
    let error = serde_json::from_str::<MCPServer>(json).unwrap_err();
    assert!(error.to_string().contains(field));
}

#[test]
fn record_minimal_json_takes_identity_defaults() {
    let server = base_server();
    assert_eq!(server.server_id, "srv");
    assert_eq!(server.name, "Server");
    assert_eq!(server.transport, MCPTransport::Http);
    assert_eq!(server.protocol_version, Some(MCPUpstreamProtocol::Auto));
    assert_eq!(server.auth_type, None);
    assert!(server.alias.is_none());
    assert!(server.server_name.is_none());
    assert!(server.url.is_none());
    assert!(server.spec_path.is_none());
    assert!(server.authentication_token.is_none());
    assert!(server.instructions.is_none());
    assert!(server.mcp_info.is_none());
}

#[test]
fn record_minimal_json_takes_catalog_defaults() {
    let server = base_server();
    assert!(server.extra_headers.is_none());
    assert!(server.allowed_tools.is_none());
    assert!(server.disallowed_tools.is_none());
    assert!(server.tool_name_to_display_name.is_none());
    assert!(server.tool_name_to_description.is_none());
    assert!(server.pinned_tools.is_none());
    assert!(server.allowed_params.is_none());
    assert!(server.static_headers.is_none());
    assert!(server.env_vars.is_none());
}

#[test]
fn record_minimal_json_takes_oauth_defaults() {
    let server = base_server();
    assert!(server.client_id.is_none());
    assert!(server.client_secret.is_none());
    assert!(server.issuer.is_none());
    assert!(!server.issuer_is_anchored);
    assert!(server.scopes.is_none());
    assert!(server.authorization_url.is_none());
    assert!(server.token_url.is_none());
    assert!(server.registration_url.is_none());
}

#[test]
fn record_minimal_json_takes_configured_oauth_defaults() {
    let server = base_server();
    assert!(server.configured_authorization_url.is_none());
    assert!(server.configured_token_url.is_none());
    assert!(server.configured_registration_url.is_none());
    assert!(server.configured_scopes.is_none());
    assert!(server.token_endpoint_auth_method.is_none());
    assert!(server.upstream_resource.is_none());
    assert!(server.upstream_token_header.is_none());
}

#[test]
fn record_minimal_json_takes_cloud_defaults() {
    let server = base_server();
    assert!(server.aws_access_key_id.is_none());
    assert!(server.aws_secret_access_key.is_none());
    assert!(server.aws_session_token.is_none());
    assert!(server.aws_region_name.is_none());
    assert!(server.aws_service_name.is_none());
    assert!(server.aws_role_name.is_none());
    assert!(server.aws_session_name.is_none());
    assert!(server.token_exchange_endpoint.is_none());
    assert!(server.audience.is_none());
}

#[test]
fn record_minimal_json_takes_token_exchange_defaults() {
    let server = base_server();
    assert_eq!(server.subject_token_type, DEFAULT_SUBJECT_TOKEN_TYPE);
    assert!(server.id_jag_resource_token_endpoint.is_none());
    assert!(server.id_jag_resource.is_none());
    assert!(server.client_private_key.is_none());
    assert!(server.client_private_key_id.is_none());
    assert_eq!(server.client_assertion_signing_alg, "RS256");
    assert_eq!(server.token_exchange_profile, "rfc8693");
}

#[test]
fn record_minimal_json_takes_stdio_defaults() {
    let server = base_server();
    assert!(server.command.is_none());
    assert!(server.args.is_none());
    assert!(server.env.is_none());
    assert!(server.access_groups.is_none());
}

#[test]
fn record_minimal_json_takes_flag_defaults() {
    let server = base_server();
    assert!(!server.allow_all_keys);
    assert!(server.available_on_public_internet);
    assert!(!server.delegate_auth_to_upstream);
    assert!(!server.oauth_passthrough);
    assert!(server.dcr_bridge.is_none());
    assert!(!server.per_server_oauth_discovery);
    assert!(!server.is_byok);
    assert!(server.byok_description.is_empty());
    assert!(server.byok_api_key_help_url.is_none());
    assert!(server.source_url.is_none());
}

#[test]
fn record_minimal_json_takes_runtime_defaults() {
    let server = base_server();
    assert!(server.created_at.is_none());
    assert!(server.updated_at.is_none());
    assert!(server.oauth2_flow.is_none());
    assert!(server.token_validation.is_none());
    assert!(server.oauth_identity_binding.is_none());
    assert!(server.token_storage_ttl_seconds.is_none());
    assert!(server.timeout.is_none());
    assert!(server.max_concurrent_requests.is_none());
    assert!(server.short_prefix.is_none());
    assert!(!server.allow_sampling);
    assert!(!server.allow_elicitation);
}

#[test]
fn record_json_round_trip_of_defaults() {
    let server = base_server();
    let encoded = serde_json::to_string(&server).unwrap();
    let decoded = server_from_json(&encoded);
    assert_eq!(decoded, server);
}

#[test]
fn record_full_json_decodes_identity_and_transport() {
    let server = server_from_json(FULL_SERVER_JSON);
    assert_eq!(server.alias.as_deref(), Some("alias"));
    assert_eq!(server.server_name.as_deref(), Some("server-name"));
    assert_eq!(server.url.as_deref(), Some("https://mcp.example"));
    assert_eq!(server.transport, MCPTransport::Stdio);
    assert_eq!(server.protocol_version, Some(MCPUpstreamProtocol::Mar2025));
    assert_eq!(server.spec_path.as_deref(), Some("/spec.json"));
    assert_eq!(server.auth_type, Some(MCPAuth::Oauth2));
    assert_eq!(server.instructions.as_deref(), Some("use it"));
    assert_eq!(server.oauth2_flow, Some(OAuth2Flow::AuthorizationCode));
    assert!(server.mcp_info.is_some());
}

#[test]
fn record_full_json_decodes_catalog() {
    let server = server_from_json(FULL_SERVER_JSON);
    assert_eq!(server.extra_headers, Some(vec!["Authorization".to_owned()]));
    assert_eq!(server.allowed_tools, Some(vec!["one".to_owned()]));
    assert_eq!(server.disallowed_tools, Some(vec!["two".to_owned()]));
    assert_eq!(
        server
            .tool_name_to_display_name
            .as_ref()
            .and_then(|names| names.get("one"))
            .map(String::as_str),
        Some("One")
    );
    assert_eq!(
        server
            .tool_name_to_description
            .as_ref()
            .and_then(|names| names.get("one"))
            .map(String::as_str),
        Some("The first")
    );
    assert!(server.pinned_tools.is_some());
    assert!(server.allowed_params.is_some());
    assert!(server.static_headers.is_some());
    assert!(server.env_vars.is_some());
}

#[test]
fn record_full_json_decodes_oauth_and_credentials() {
    let server = server_from_json(FULL_SERVER_JSON);
    assert_eq!(server.client_id.as_deref(), Some("client"));
    assert!(server.issuer_is_anchored);
    assert_eq!(server.scopes, Some(vec!["read".to_owned()]));
    assert_eq!(
        server.authorization_url.as_deref(),
        Some("https://idp.example/authorize")
    );
    assert_eq!(server.configured_scopes, Some(vec!["openid".to_owned()]));
    assert_eq!(
        server.upstream_resource.as_deref(),
        Some("https://mcp.example")
    );
    assert_eq!(server.upstream_token_header.as_deref(), Some("X-Upstream"));
    assert_eq!(server.aws_region_name.as_deref(), Some("us-east-1"));
    assert_eq!(
        server.aws_service_name.as_deref(),
        Some("bedrock-agentcore")
    );
    assert_eq!(
        server.token_exchange_endpoint.as_deref(),
        Some("https://idp.example/exchange")
    );
}

#[test]
fn record_full_json_decodes_token_exchange_and_limits() {
    let server = server_from_json(FULL_SERVER_JSON);
    assert_eq!(server.audience.as_deref(), Some("api://audience"));
    assert_eq!(
        server.subject_token_type,
        "urn:ietf:params:oauth:token-type:id_token"
    );
    assert_eq!(server.id_jag_resource.as_deref(), Some("api://resource"));
    assert_eq!(server.client_private_key_id.as_deref(), Some("key-id"));
    assert_eq!(server.client_assertion_signing_alg, "ES256");
    assert_eq!(server.token_exchange_profile, "entra_obo");
    assert_eq!(server.command.as_deref(), Some("mcp-server"));
    assert_eq!(server.args, Some(vec!["--stdio".to_owned()]));
    assert_eq!(serde_json::to_string(&server.timeout).unwrap(), "12.5");
    assert_eq!(server.max_concurrent_requests, Some(4));
    assert_eq!(server.short_prefix.as_deref(), Some("abc"));
}

#[test]
fn record_full_json_decodes_flags_and_binding() {
    let server = server_from_json(FULL_SERVER_JSON);
    assert!(server.allow_all_keys);
    assert!(!server.available_on_public_internet);
    assert!(server.oauth_passthrough);
    assert!(server.dcr_bridge.unwrap());
    assert!(server.per_server_oauth_discovery);
    assert!(server.is_byok);
    assert_eq!(server.byok_description, vec!["bring a key".to_owned()]);
    assert!(server.created_at.is_some());
    assert!(server.updated_at.is_some());
    assert!(server.allow_sampling);
    assert!(server.allow_elicitation);
    assert_eq!(
        server.oauth_identity_binding.unwrap().mode,
        BindingMode::Audit
    );
}

#[test]
fn record_secret_fields_use_the_secret_wrapper() {
    let server = server_from_json(SECRET_SERVER_JSON);
    let secrets = [
        server
            .authentication_token
            .as_ref()
            .unwrap()
            .0
            .0
            .expose_secret(),
        server.client_secret.as_ref().unwrap().0.0.expose_secret(),
        server
            .aws_session_token
            .as_ref()
            .unwrap()
            .0
            .0
            .expose_secret(),
    ];
    assert_eq!(secrets, ["tok-1", "sec-1", "session-1"]);
}

#[test]
fn record_secret_cloud_fields_use_the_secret_wrapper() {
    let server = server_from_json(SECRET_SERVER_JSON);
    let secrets = [
        server
            .aws_access_key_id
            .as_ref()
            .unwrap()
            .0
            .0
            .expose_secret(),
        server
            .aws_secret_access_key
            .as_ref()
            .unwrap()
            .0
            .0
            .expose_secret(),
        server
            .client_private_key
            .as_ref()
            .unwrap()
            .0
            .0
            .expose_secret(),
    ];
    assert_eq!(secrets, ["AKIA1", "aws-1", "key-1"]);
}

#[test]
fn record_debug_and_display_print_only_identity_fields() {
    let server = server_from_json(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "auth_type": "oauth2",
            "authentication_token": "tok-secret",
            "client_secret": "client-secret",
            "aws_secret_access_key": "aws-secret"
        }"#,
    );
    let debug = format!("{server:?}");
    let display = format!("{server}");
    assert_eq!(debug, display);
    assert_eq!(
        debug,
        r#"MCPServer(server_id="srv", name="Server", transport=Http, auth_type=Some(Oauth2))"#
    );
    assert!(!debug.contains("tok-secret"));
    assert!(!debug.contains("client-secret"));
    assert!(!debug.contains("aws-secret"));
}

#[rstest]
#[case::client_credentials("client_credentials", OAuth2Flow::ClientCredentials)]
#[case::authorization_code("authorization_code", OAuth2Flow::AuthorizationCode)]
fn record_oauth2_flow_parses_and_serializes(#[case] text: &str, #[case] flow: OAuth2Flow) {
    let parsed: OAuth2Flow = serde_json::from_str(&format!("\"{text}\"")).unwrap();
    assert_eq!(parsed, flow);
    assert_eq!(serde_json::to_string(&flow).unwrap(), format!("\"{text}\""));
}

#[rstest]
#[case::implicit(r#""implicit""#)]
#[case::camel_case(r#""authorizationCode""#)]
fn record_oauth2_flow_rejects_other_text(#[case] text: &str) {
    let error = serde_json::from_str::<OAuth2Flow>(text).unwrap_err();
    assert!(error.to_string().contains("Matching variant not found"));
}

#[test]
fn record_oauth2_flow_is_used_by_the_server() {
    let server = server_from_json(
        r#"{"server_id": "srv", "name": "Server", "transport": "http", "oauth2_flow": "client_credentials"}"#,
    );
    assert_eq!(server.oauth2_flow, Some(OAuth2Flow::ClientCredentials));
}

#[test]
fn record_oauth_metadata_defaults() {
    let metadata: MCPOAuthMetadata = serde_json::from_str("{}").unwrap();
    assert!(metadata.scopes.is_none());
    assert!(metadata.authorization_url.is_none());
    assert!(metadata.token_url.is_none());
    assert!(metadata.registration_url.is_none());
    assert!(metadata.discovered_issuer.is_none());
    assert!(!metadata.from_origin_fallback);
}

#[test]
fn record_oauth_metadata_decodes_every_field() {
    let json = r#"{
        "scopes": ["read"],
        "authorization_url": "https://idp.example/authorize",
        "token_url": "https://idp.example/token",
        "registration_url": "https://idp.example/register",
        "discovered_issuer": "https://idp.example",
        "from_origin_fallback": true
    }"#;
    let metadata: MCPOAuthMetadata = serde_json::from_str(json).unwrap();
    assert_eq!(metadata.scopes, Some(vec!["read".to_owned()]));
    assert_eq!(
        metadata.authorization_url.as_deref(),
        Some("https://idp.example/authorize")
    );
    assert_eq!(
        metadata.token_url.as_deref(),
        Some("https://idp.example/token")
    );
    assert_eq!(
        metadata.registration_url.as_deref(),
        Some("https://idp.example/register")
    );
    assert_eq!(
        metadata.discovered_issuer.as_deref(),
        Some("https://idp.example")
    );
    assert!(metadata.from_origin_fallback);
}

#[test]
fn record_identity_binding_defaults() {
    let binding: MCPOAuthIdentityBinding =
        serde_json::from_str(r#"{"issuer": "https://idp.example", "audiences": ["aud"]}"#).unwrap();
    assert_eq!(binding.mode, BindingMode::Disabled);
    assert_eq!(binding.issuer, "https://idp.example");
    assert!(binding.jwks_url.is_none());
    assert_eq!(binding.audiences, vec!["aud".to_owned()]);
    assert_eq!(binding.principal_claim, "email");
    assert_eq!(binding.caller_field, CallerField::UserEmail);
    assert!(binding.require_email_verified);
}

#[test]
fn record_identity_binding_wire_values() {
    let json = r#"{
        "mode": "audit",
        "issuer": "https://idp.example",
        "audiences": ["aud"],
        "caller_field": "user_id",
        "require_email_verified": false
    }"#;
    let binding: MCPOAuthIdentityBinding = serde_json::from_str(json).unwrap();
    assert_eq!(binding.mode, BindingMode::Audit);
    assert_eq!(binding.caller_field, CallerField::UserId);
    assert!(!binding.require_email_verified);
    let encoded = serde_json::to_string(&binding).unwrap();
    assert!(encoded.contains(r#""mode":"audit""#));
    assert!(encoded.contains(r#""caller_field":"user_id""#));
    let disabled: MCPOAuthIdentityBinding = serde_json::from_str(
        r#"{"issuer": "https://idp.example", "audiences": ["aud"], "mode": "disabled"}"#,
    )
    .unwrap();
    assert_eq!(disabled.mode, BindingMode::Disabled);
}

#[test]
fn record_identity_binding_rejects_unknown_mode() {
    let error = serde_json::from_str::<MCPOAuthIdentityBinding>(
        r#"{"issuer": "https://idp.example", "audiences": ["aud"], "mode": "log"}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn validate_upstream_token_header_trims_valid_names() {
    let server = server_from_json(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "upstream_token_header": "  X-Upstream-Token  "
        }"#,
    );
    assert_eq!(
        server.upstream_token_header.as_deref(),
        Some("X-Upstream-Token")
    );
}

#[test]
fn validate_upstream_token_header_nulls_blank_values() {
    let server = server_from_json(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "upstream_token_header": "   "
        }"#,
    );
    assert!(server.upstream_token_header.is_none());
}

#[test]
fn validate_upstream_token_header_rejects_invalid_names() {
    let error = serde_json::from_str::<MCPServer>(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "upstream_token_header": "bad header"
        }"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("invalid upstream header name"));
}

#[test]
fn validate_runs_the_header_check_before_the_binding_check() {
    let error = serde_json::from_str::<MCPServer>(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "upstream_token_header": "bad header",
            "auth_type": "oauth2",
            "oauth2_flow": "client_credentials",
            "oauth_identity_binding": {
                "mode": "enforce",
                "issuer": "https://idp.example",
                "audiences": ["aud"]
            }
        }"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("invalid upstream header name"));
}

#[test]
fn validate_protocol_version_defaults_to_auto_without_mcp_info() {
    assert_eq!(
        base_server().protocol_version,
        Some(MCPUpstreamProtocol::Auto)
    );
}

#[test]
fn validate_protocol_version_defaults_to_auto_when_mcp_info_has_none() {
    let server = server_from_json(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "mcp_info": {"owner": "team"}
        }"#,
    );
    assert_eq!(server.protocol_version, Some(MCPUpstreamProtocol::Auto));
}

#[test]
fn validate_protocol_version_comes_from_mcp_info_when_not_given() {
    let server = server_from_json(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "mcp_info": {"protocol_version": "2025-06-18"}
        }"#,
    );
    assert_eq!(server.protocol_version, Some(MCPUpstreamProtocol::Jun2025));
}

#[test]
fn validate_explicit_protocol_version_wins_over_mcp_info() {
    let server = server_from_json(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "protocol_version": "2024-11-05",
            "mcp_info": {"protocol_version": "2025-06-18"}
        }"#,
    );
    assert_eq!(server.protocol_version, Some(MCPUpstreamProtocol::Nov2024));
}

#[rstest]
#[case::unknown_version(r#"{"protocol_version": "2026-07-28"}"#)]
#[case::explicit_null(r#"{"protocol_version": null}"#)]
#[case::non_text(r#"{"protocol_version": 20250618}"#)]
fn validate_rejects_invalid_protocol_version_in_mcp_info(#[case] mcp_info: &str) {
    let json = format!(
        r#"{{"server_id": "srv", "name": "Server", "transport": "http", "mcp_info": {mcp_info}}}"#
    );
    let error = serde_json::from_str::<MCPServer>(&json).unwrap_err();
    assert!(error.to_string().contains("invalid MCP record"));
}

#[test]
fn validate_identity_binding_requires_gateway_managed_user_tokens() {
    let error = serde_json::from_str::<MCPServer>(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "auth_type": "oauth2",
            "oauth2_flow": "client_credentials",
            "oauth_identity_binding": {
                "mode": "enforce",
                "issuer": "https://idp.example",
                "audiences": ["aud"]
            }
        }"#,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains(
            "oauth_identity_binding requires gateway-managed per-user OAuth2 credentials"
        )
    );
}

#[test]
fn validate_identity_binding_rejects_delegated_upstream_auth() {
    let error = serde_json::from_str::<MCPServer>(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "auth_type": "oauth2",
            "delegate_auth_to_upstream": true,
            "oauth_identity_binding": {
                "mode": "audit",
                "issuer": "https://idp.example",
                "audiences": ["aud"]
            }
        }"#,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains(
            "oauth_identity_binding requires gateway-managed per-user OAuth2 credentials"
        )
    );
}

#[test]
fn validate_identity_binding_accepts_enforce_for_user_oauth2() {
    let server = server_from_json(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "auth_type": "oauth2",
            "oauth_identity_binding": {
                "mode": "enforce",
                "issuer": "https://idp.example",
                "audiences": ["aud"]
            }
        }"#,
    );
    assert_eq!(
        server.oauth_identity_binding.unwrap().mode,
        BindingMode::Enforce
    );
}

#[test]
fn validate_identity_binding_disabled_needs_no_user_oauth() {
    let server = server_from_json(
        r#"{
            "server_id": "srv",
            "name": "Server",
            "transport": "http",
            "auth_type": "api_key",
            "oauth_identity_binding": {
                "mode": "disabled",
                "issuer": "https://idp.example",
                "audiences": ["aud"]
            }
        }"#,
    );
    assert_eq!(
        server.oauth_identity_binding.unwrap().mode,
        BindingMode::Disabled
    );
}

#[test]
fn validate_identity_binding_rejects_empty_audiences() {
    let error = serde_json::from_str::<MCPOAuthIdentityBinding>(
        r#"{"issuer": "https://idp.example", "audiences": []}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("audiences must not be empty"));
}

#[test]
fn prop_effective_authorization_url() {
    let mut server = base_server();
    server.authorization_url = Some("https://live.example".to_owned());
    server.configured_authorization_url = Some("https://configured.example".to_owned());
    assert_eq!(
        server.effective_authorization_url(),
        Some("https://live.example")
    );
    server.authorization_url = Some(String::new());
    assert_eq!(
        server.effective_authorization_url(),
        Some("https://configured.example")
    );
    server.authorization_url = None;
    assert_eq!(
        server.effective_authorization_url(),
        Some("https://configured.example")
    );
    server.configured_authorization_url = None;
    assert_eq!(server.effective_authorization_url(), None);
    server.authorization_url = Some(String::new());
    server.configured_authorization_url = Some(String::new());
    assert_eq!(server.effective_authorization_url(), Some(""));
}

#[test]
fn prop_effective_token_url() {
    let mut server = base_server();
    server.token_url = Some("https://live.example".to_owned());
    server.configured_token_url = Some("https://configured.example".to_owned());
    assert_eq!(server.effective_token_url(), Some("https://live.example"));
    server.token_url = Some(String::new());
    assert_eq!(
        server.effective_token_url(),
        Some("https://configured.example")
    );
    server.token_url = None;
    assert_eq!(
        server.effective_token_url(),
        Some("https://configured.example")
    );
    server.configured_token_url = None;
    assert_eq!(server.effective_token_url(), None);
}

#[test]
fn prop_effective_registration_url() {
    let mut server = base_server();
    server.registration_url = Some("https://live.example".to_owned());
    server.configured_registration_url = Some("https://configured.example".to_owned());
    assert_eq!(
        server.effective_registration_url(),
        Some("https://live.example")
    );
    server.registration_url = Some(String::new());
    assert_eq!(
        server.effective_registration_url(),
        Some("https://configured.example")
    );
    server.registration_url = None;
    assert_eq!(
        server.effective_registration_url(),
        Some("https://configured.example")
    );
    server.configured_registration_url = None;
    assert_eq!(server.effective_registration_url(), None);
}

#[test]
fn prop_has_client_credentials() {
    let mut server = base_server();
    server.oauth2_flow = Some(OAuth2Flow::ClientCredentials);
    assert!(server.has_client_credentials());
    server.oauth2_flow = Some(OAuth2Flow::AuthorizationCode);
    assert!(!server.has_client_credentials());
    server.oauth2_flow = None;
    assert!(!server.has_client_credentials());
}

#[test]
fn prop_needs_user_oauth_token() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::Oauth2);
    assert!(server.needs_user_oauth_token());
    server.oauth2_flow = Some(OAuth2Flow::ClientCredentials);
    assert!(!server.needs_user_oauth_token());
    server.oauth2_flow = Some(OAuth2Flow::AuthorizationCode);
    assert!(server.needs_user_oauth_token());
    server.auth_type = Some(MCPAuth::ApiKey);
    server.oauth2_flow = None;
    assert!(!server.needs_user_oauth_token());
    server.auth_type = None;
    assert!(!server.needs_user_oauth_token());
}

#[test]
fn prop_is_gateway_managed_oauth2() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::Oauth2);
    assert!(server.is_gateway_managed_oauth2());
    server.delegate_auth_to_upstream = true;
    assert!(!server.is_gateway_managed_oauth2());
    server.delegate_auth_to_upstream = false;
    server.auth_type = Some(MCPAuth::ApiKey);
    assert!(!server.is_gateway_managed_oauth2());
    server.auth_type = None;
    assert!(!server.is_gateway_managed_oauth2());
}

#[test]
fn prop_uses_per_server_oauth_relay() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::Oauth2);
    server.per_server_oauth_discovery = true;
    assert!(server.uses_per_server_oauth_relay());
    server.per_server_oauth_discovery = false;
    assert!(!server.uses_per_server_oauth_relay());
    server.per_server_oauth_discovery = true;
    server.oauth2_flow = Some(OAuth2Flow::ClientCredentials);
    assert!(!server.uses_per_server_oauth_relay());
    server.oauth2_flow = None;
    server.auth_type = Some(MCPAuth::ApiKey);
    assert!(!server.uses_per_server_oauth_relay());
}

#[test]
fn prop_advertises_gateway_authorization_server() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::Oauth2);
    assert!(server.advertises_gateway_authorization_server());
    server.per_server_oauth_discovery = true;
    assert!(!server.advertises_gateway_authorization_server());
    server.per_server_oauth_discovery = false;
    server.delegate_auth_to_upstream = true;
    assert!(!server.advertises_gateway_authorization_server());
    server.delegate_auth_to_upstream = false;
    server.auth_type = Some(MCPAuth::ApiKey);
    assert!(server.advertises_gateway_authorization_server());
    server.auth_type = Some(MCPAuth::None);
    assert!(server.advertises_gateway_authorization_server());
    server.auth_type = None;
    assert!(server.advertises_gateway_authorization_server());
    let mut header_server = with_headers(base_server(), &["Authorization"]);
    header_server.auth_type = Some(MCPAuth::ApiKey);
    assert!(!header_server.advertises_gateway_authorization_server());
    let mut apikey_server = with_headers(base_server(), &["apikey"]);
    apikey_server.auth_type = Some(MCPAuth::BearerToken);
    assert!(!apikey_server.advertises_gateway_authorization_server());
    let mut x_api_key_server = with_headers(base_server(), &["x-api-key"]);
    assert!(!x_api_key_server.advertises_gateway_authorization_server());
    x_api_key_server.auth_type = Some(MCPAuth::TruePassthrough);
    assert!(!x_api_key_server.advertises_gateway_authorization_server());
    x_api_key_server.auth_type = Some(MCPAuth::Oauth2TokenExchange);
    assert!(!x_api_key_server.advertises_gateway_authorization_server());
    x_api_key_server.auth_type = Some(MCPAuth::OauthDelegate);
    assert!(!x_api_key_server.advertises_gateway_authorization_server());
}

#[test]
fn prop_is_true_passthrough() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::TruePassthrough);
    assert!(server.is_true_passthrough());
    server.auth_type = Some(MCPAuth::OauthDelegate);
    assert!(!server.is_true_passthrough());
    server.auth_type = Some(MCPAuth::Oauth2);
    assert!(!server.is_true_passthrough());
    server.auth_type = None;
    assert!(!server.is_true_passthrough());
}

#[test]
fn prop_is_oauth_delegate() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::OauthDelegate);
    assert!(server.is_oauth_delegate());
    server.auth_type = Some(MCPAuth::TruePassthrough);
    assert!(!server.is_oauth_delegate());
    server.auth_type = Some(MCPAuth::ApiKey);
    assert!(!server.is_oauth_delegate());
}

#[test]
fn prop_is_client_forwarded_token() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::TruePassthrough);
    assert!(server.is_client_forwarded_token());
    server.auth_type = Some(MCPAuth::OauthDelegate);
    assert!(server.is_client_forwarded_token());
    server.auth_type = Some(MCPAuth::Oauth2);
    assert!(!server.is_client_forwarded_token());
    server.auth_type = None;
    assert!(!server.is_client_forwarded_token());
}

#[test]
fn prop_is_dcr_bridge() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::TruePassthrough);
    server.dcr_bridge = Some(true);
    assert!(server.is_dcr_bridge());
    server.auth_type = Some(MCPAuth::OauthDelegate);
    assert!(server.is_dcr_bridge());
    server.auth_type = Some(MCPAuth::Oauth2);
    assert!(!server.is_dcr_bridge());
    server.auth_type = Some(MCPAuth::TruePassthrough);
    server.dcr_bridge = Some(false);
    assert!(!server.is_dcr_bridge());
    server.dcr_bridge = None;
    assert!(!server.is_dcr_bridge());
}

#[test]
fn prop_requires_per_user_auth() {
    let mut server = base_server();
    server.auth_type = Some(MCPAuth::Oauth2);
    assert!(server.requires_per_user_auth());
    server.oauth2_flow = Some(OAuth2Flow::ClientCredentials);
    assert!(!server.requires_per_user_auth());
    server.auth_type = Some(MCPAuth::None);
    server.oauth2_flow = None;
    assert!(!server.requires_per_user_auth());
    let mut header_server = with_headers(base_server(), &["Authorization"]);
    header_server.auth_type = Some(MCPAuth::None);
    assert!(header_server.requires_per_user_auth());
    let mut x_api_key_server = with_headers(base_server(), &["X-API-KEY"]);
    x_api_key_server.auth_type = Some(MCPAuth::None);
    assert!(x_api_key_server.requires_per_user_auth());
    let null_auth_server = with_headers(base_server(), &["Authorization"]);
    assert!(!null_auth_server.requires_per_user_auth());
    let mut api_key_server = with_headers(base_server(), &["Authorization"]);
    api_key_server.auth_type = Some(MCPAuth::ApiKey);
    assert!(!api_key_server.requires_per_user_auth());
    let mut passthrough_server = base_server();
    passthrough_server.auth_type = Some(MCPAuth::TruePassthrough);
    assert!(passthrough_server.requires_per_user_auth());
}

#[test]
fn prop_is_oauth_passthrough() {
    let mut opted_in_server = with_headers(base_server(), &["Authorization"]);
    opted_in_server.oauth_passthrough = true;
    assert!(opted_in_server.is_oauth_passthrough());
    opted_in_server.oauth_passthrough = false;
    assert!(!opted_in_server.is_oauth_passthrough());
    opted_in_server.oauth_passthrough = true;
    opted_in_server.auth_type = Some(MCPAuth::None);
    assert!(opted_in_server.is_oauth_passthrough());
    let mut lowercase_server = with_headers(base_server(), &["authorization"]);
    lowercase_server.auth_type = Some(MCPAuth::None);
    lowercase_server.oauth_passthrough = true;
    assert!(lowercase_server.is_oauth_passthrough());
    let mut api_key_server = with_headers(base_server(), &["x-api-key"]);
    api_key_server.auth_type = Some(MCPAuth::None);
    api_key_server.oauth_passthrough = true;
    assert!(!api_key_server.is_oauth_passthrough());
    let mut oauth2_server = with_headers(base_server(), &["Authorization"]);
    oauth2_server.auth_type = Some(MCPAuth::Oauth2);
    oauth2_server.oauth_passthrough = true;
    assert!(!oauth2_server.is_oauth_passthrough());
    let mut bare_server = base_server();
    bare_server.oauth_passthrough = true;
    assert!(!bare_server.is_oauth_passthrough());
}

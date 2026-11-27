#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::{McpServerTable, McpStatus};
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth, MCPCredentials, MCPTransport};
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_payload::request::NewMCPServerRequest;
use betterlite_mcp::server_record::MCPServer;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::sanitize_fields::reduce_publicity;
use super::{
    ADMIN_REQUIRED, CredentialTargetError, INHERITED_CREDENTIAL_FIELDS, MISSING_USER_ID,
    has_non_admin_config_credentials, inherit_credentials_from_existing_server,
    redact_mcp_credentials_list, resolve_credential_target_user_id,
    sanitize_mcp_server_for_non_admin, sanitize_mcp_server_for_virtual_key,
    sanitize_mcp_server_list_for_non_admin, sanitize_mcp_server_list_for_virtual_key,
    user_is_full_admin,
};

const PARENT_SERVER: &str = r#"{
    "server_id": "srv-1",
    "name": "Parent Server",
    "transport": "http",
    "authentication_token": "parent-auth-token",
    "client_id": "parent-client-id",
    "client_secret": "parent-client-secret",
    "scopes": ["read", "write"],
    "aws_access_key_id": "parent-access-key",
    "aws_secret_access_key": "parent-secret-key",
    "aws_session_token": "parent-session-token",
    "aws_region_name": "us-east-1",
    "aws_service_name": "bedrock-agentcore",
    "upstream_resource": "https://resource.example.test",
    "upstream_token_header": "X-Upstream-Token"
}"#;

const FULL_ROW: &str = r#"{
    "server_id": "srv-1",
    "server_name": "Test Server",
    "alias": "test-alias",
    "description": "test description",
    "url": "https://mcp.example.test/sse?key=test-url-secret",
    "spec_path": "https://mcp.example.test/openapi.json?key=test-spec-secret",
    "transport": "http",
    "auth_type": "bearer_token",
    "credentials": {
        "auth_value": "test-auth-secret",
        "client_secret": "test-client-secret",
        "upstream_resource": "https://resource.example.test",
        "scopes": ["read"]
    },
    "instructions": "test instructions",
    "created_at": "2025-01-01T00:00:00Z",
    "created_by": "test-creator",
    "updated_at": "2025-01-02T00:00:00Z",
    "updated_by": "test-updater",
    "teams": [{"team_id": "team-1"}],
    "mcp_access_groups": ["group-1"],
    "allowed_tools": ["tool-1"],
    "tool_name_to_display_name": {"tool-1": "Tool One"},
    "tool_name_to_description": {"tool-1": "does things"},
    "pinned_tools": {"tool-1": {"description": "a pinned tool", "input_schema": {}}},
    "extra_headers": ["X-Test: test-extra"],
    "mcp_info": {"protocol_version": "2025-06-18", "owner": "test-owner"},
    "static_headers": {"X-Api-Key": "test-static-secret"},
    "env_vars": [{"name": "API_KEY", "value": "test-env-secret", "scope": "global"}],
    "status": "healthy",
    "last_health_check": "2025-01-03T00:00:00Z",
    "health_check_error": "test health failure",
    "command": "npx",
    "args": ["-y", "test-server"],
    "env": {"DB_PASSWORD": "test-db-secret"},
    "issuer": "https://issuer.example.test",
    "authorization_url": "https://auth.example.test/authorize",
    "token_url": "https://auth.example.test/token",
    "registration_url": "https://auth.example.test/register",
    "oauth2_flow": "client_credentials",
    "token_exchange_endpoint": "https://auth.example.test/exchange",
    "audience": "test-audience",
    "subject_token_type": "urn:ietf:params:oauth:token-type:access_token",
    "token_exchange_profile": "rfc8693"
}"#;

fn row(source: &str) -> McpServerTable {
    serde_json::from_str(source).unwrap()
}

fn server(source: &str) -> MCPServer {
    serde_json::from_str(source).unwrap()
}

fn payload(source: &str) -> NewMCPServerRequest {
    serde_json::from_str(source).unwrap()
}

fn decoded_credentials(source: &str) -> MCPCredentials {
    serde_json::from_str(source).unwrap()
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn registry(public_ids: &[String], strict: bool, db_servers: Vec<MCPServer>) -> ServerRegistry {
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: public_ids.to_vec(),
            public_hub_strict: strict,
            internal_ip_ranges: None,
        },
        Vec::new(),
        db_servers,
    )
}

fn auth(source: &str) -> UserAPIKeyAuth {
    let JsonData::Object(fields) = serde_json::from_str(source).unwrap() else {
        panic!("an auth fixture is an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn fields_of(credentials: &MCPCredentials) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(&serde_json::to_string(credentials).unwrap()).unwrap()
}

fn request_fields(request: &NewMCPServerRequest) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(&serde_json::to_string(request).unwrap()).unwrap()
}

fn json_fields(source: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(source).unwrap()
}

fn info_fields(view: &McpServerTable) -> BTreeMap<String, JsonValue> {
    let info = view.mcp_info.as_ref().expect("the view carries info");
    serde_json::from_str(&serde_json::to_string(info).unwrap()).unwrap()
}

fn non_admin(source: &str) -> McpServerTable {
    sanitize_mcp_server_for_non_admin(&row(source), &registry(&[], false, Vec::new()), &[]).unwrap()
}

fn virtual_key(source: &str) -> McpServerTable {
    sanitize_mcp_server_for_virtual_key(&row(source), &registry(&[], false, Vec::new()), &[])
        .unwrap()
}

fn assert_shared_stripped(view: &McpServerTable) {
    assert!(view.credentials.is_none());
    assert!(view.pinned_tools.is_none());
    assert!(view.url.is_none());
    assert!(view.static_headers.is_none());
    assert!(view.extra_headers.is_empty());
    assert!(view.env.0.is_empty());
    assert!(view.command.is_none());
    assert!(view.args.is_empty());
    assert!(view.issuer.is_none());
    assert!(view.authorization_url.is_none());
    assert!(view.token_url.is_none());
    assert!(view.registration_url.is_none());
    assert!(view.token_exchange_endpoint.is_none());
    assert!(view.audience.is_none());
    assert!(view.subject_token_type.is_none());
    assert!(view.token_exchange_profile.is_none());
    assert!(view.env_vars.is_none());
}

#[rstest]
#[case::proxy_admin(r#"{"user_role": "proxy_admin"}"#, true)]
#[case::proxy_admin_viewer(r#"{"user_role": "proxy_admin_viewer"}"#, false)]
#[case::org_admin(r#"{"user_role": "org_admin"}"#, false)]
#[case::internal_user(r#"{"user_role": "internal_user"}"#, false)]
#[case::internal_user_viewer(r#"{"user_role": "internal_user_viewer"}"#, false)]
#[case::team(r#"{"user_role": "team"}"#, false)]
#[case::customer(r#"{"user_role": "customer"}"#, false)]
#[case::no_role(r#"{"user_id": "caller-1"}"#, false)]
fn full_admin_only_for_proxy_admin(#[case] fixture: &str, #[case] expected: bool) {
    assert_eq!(user_is_full_admin(&auth(fixture)), expected);
}

#[test]
fn credential_target_defaults_to_the_caller() {
    let user = auth(r#"{"user_id": "caller-1", "user_role": "internal_user"}"#);
    assert_eq!(
        resolve_credential_target_user_id(&user, None).unwrap(),
        "caller-1"
    );
    assert_eq!(
        resolve_credential_target_user_id(&user, Some("caller-1")).unwrap(),
        "caller-1"
    );
}

#[rstest]
#[case::viewer(r#"{"user_id": "admin-1", "user_role": "proxy_admin_viewer"}"#)]
#[case::internal_user(r#"{"user_id": "caller-1", "user_role": "internal_user"}"#)]
#[case::no_role(r#"{"user_id": "caller-1"}"#)]
fn credential_target_rejects_another_user(#[case] fixture: &str) {
    let error = resolve_credential_target_user_id(&auth(fixture), Some("user-2")).unwrap_err();
    assert_eq!(error, CredentialTargetError::AdminRequired);
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
}

#[test]
fn credential_target_admin_names_another_user() {
    let admin = auth(r#"{"user_id": "admin-1", "user_role": "proxy_admin"}"#);
    assert_eq!(
        resolve_credential_target_user_id(&admin, Some("user-2")).unwrap(),
        "user-2"
    );
}

#[test]
fn credential_target_requires_a_caller_id() {
    let anonymous = auth(r#"{"user_role": "internal_user"}"#);
    let error = resolve_credential_target_user_id(&anonymous, None).unwrap_err();
    assert_eq!(error, CredentialTargetError::MissingUserId);
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    let empty_request = resolve_credential_target_user_id(&anonymous, Some("")).unwrap_err();
    assert_eq!(empty_request, CredentialTargetError::MissingUserId);
}

#[test]
fn credential_target_admin_without_a_caller_id() {
    let admin = auth(r#"{"user_role": "proxy_admin"}"#);
    assert_eq!(
        resolve_credential_target_user_id(&admin, Some("user-2")).unwrap(),
        "user-2"
    );
}

#[test]
fn credential_target_admin_empty_request() {
    let admin = auth(r#"{"user_id": "admin-1", "user_role": "proxy_admin"}"#);
    assert_eq!(
        resolve_credential_target_user_id(&admin, Some("")).unwrap(),
        ""
    );
    let user = auth(r#"{"user_id": "caller-1", "user_role": "internal_user"}"#);
    let error = resolve_credential_target_user_id(&user, Some("")).unwrap_err();
    assert_eq!(error, CredentialTargetError::AdminRequired);
}

#[test]
fn credential_target_error_shape() {
    let forbidden = CredentialTargetError::AdminRequired;
    assert_eq!(forbidden.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(forbidden.to_string(), ADMIN_REQUIRED);
    assert_eq!(
        forbidden.detail(),
        BTreeMap::from([("error".to_owned(), text(ADMIN_REQUIRED))])
    );
    let missing = CredentialTargetError::MissingUserId;
    assert_eq!(missing.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(missing.to_string(), MISSING_USER_ID);
    assert_eq!(
        missing.detail(),
        BTreeMap::from([("error".to_owned(), text(MISSING_USER_ID))])
    );
}

#[test]
fn absent_credentials_are_not_a_credential() {
    assert!(!has_non_admin_config_credentials(None));
}

#[rstest]
#[case::upstream_resource_only(r#"{"upstream_resource": "https://resource.test"}"#)]
#[case::upstream_token_header_only(r#"{"upstream_token_header": "X-Upstream-Token"}"#)]
#[case::scopes_only(r#"{"scopes": ["read"]}"#)]
#[case::admin_config_and_scopes(
    r#"{"upstream_resource": "https://resource.test", "upstream_token_header": "X-Token", "scopes": ["read"]}"#
)]
#[case::empty(r#"{}"#)]
#[case::explicit_null(r#"{"auth_value": null, "client_id": null}"#)]
#[case::empty_string(r#"{"client_id": "", "aws_region_name": ""}"#)]
#[case::empty_list(r#"{"scopes": [], "redirect_uris": []}"#)]
#[case::empty_admin_config(r#"{"upstream_resource": ""}"#)]
fn admin_config_and_scopes_are_not_credentials(#[case] fixture: &str) {
    let supplied = decoded_credentials(fixture);
    assert!(!has_non_admin_config_credentials(Some(&supplied)));
}

#[rstest]
#[case::auth_value(r#"{"auth_value": "test-secret"}"#)]
#[case::client_id(r#"{"client_id": "test-client"}"#)]
#[case::client_secret(r#"{"client_secret": "test-secret"}"#)]
#[case::aws_access_key_id(r#"{"aws_access_key_id": "test-key"}"#)]
#[case::aws_secret_access_key(r#"{"aws_secret_access_key": "test-secret"}"#)]
#[case::aws_session_token(r#"{"aws_session_token": "test-token"}"#)]
#[case::aws_region_name(r#"{"aws_region_name": "us-east-1"}"#)]
#[case::aws_service_name(r#"{"aws_service_name": "bedrock-agentcore"}"#)]
#[case::aws_role_name(r#"{"aws_role_name": "arn:aws:iam::123456789012:role/Test"}"#)]
#[case::aws_session_name(r#"{"aws_session_name": "test-session"}"#)]
#[case::audience(r#"{"audience": "test-audience"}"#)]
#[case::token_exchange_endpoint(r#"{"token_exchange_endpoint": "https://auth.test/token"}"#)]
#[case::subject_token_type(r#"{"subject_token_type": "urn:test"}"#)]
#[case::id_jag_endpoint(r#"{"id_jag_resource_token_endpoint": "https://auth.test/id-jag"}"#)]
#[case::id_jag_resource(r#"{"id_jag_resource": "https://resource.test"}"#)]
#[case::client_private_key(r#"{"client_private_key": "test-key"}"#)]
#[case::client_private_key_id(r#"{"client_private_key_id": "test-key-id"}"#)]
#[case::signing_alg(r#"{"client_assertion_signing_alg": "RS256"}"#)]
#[case::auth_method(r#"{"token_endpoint_auth_method": "client_secret_post"}"#)]
#[case::redirect_uris(r#"{"redirect_uris": ["https://app.test/callback"]}"#)]
#[case::token_exchange_profile(r#"{"token_exchange_profile": "rfc8693"}"#)]
fn every_other_credential_field_counts(#[case] fixture: &str) {
    let supplied = decoded_credentials(fixture);
    assert!(has_non_admin_config_credentials(Some(&supplied)));
}

#[test]
fn inherits_every_listed_field() {
    let parent = registry(&[], false, vec![server(PARENT_SERVER)]);
    let request = payload(r#"{"server_id": "srv-1"}"#);
    let inherited = inherit_credentials_from_existing_server(&request, &parent);
    let merged = inherited
        .credentials
        .expect("the parent fills the credentials");
    let fields = fields_of(&merged);
    let expected = json_fields(
        r#"{
            "auth_value": "parent-auth-token",
            "client_id": "parent-client-id",
            "client_secret": "parent-client-secret",
            "scopes": ["read", "write"],
            "aws_access_key_id": "parent-access-key",
            "aws_secret_access_key": "parent-secret-key",
            "aws_session_token": "parent-session-token",
            "aws_region_name": "us-east-1",
            "aws_service_name": "bedrock-agentcore",
            "upstream_resource": "https://resource.example.test",
            "upstream_token_header": "X-Upstream-Token"
        }"#,
    );
    assert_eq!(fields.len(), INHERITED_CREDENTIAL_FIELDS.len());
    assert!(
        INHERITED_CREDENTIAL_FIELDS
            .iter()
            .all(|(_, key)| expected.contains_key(*key))
    );
    assert_eq!(fields, expected);
}

#[test]
fn admin_config_wins_over_inherited() {
    let parent = registry(&[], false, vec![server(PARENT_SERVER)]);
    let request = payload(
        r#"{"server_id": "srv-1", "credentials": {"upstream_resource": "https://payload.example.test", "upstream_token_header": "X-Payload-Token"}}"#,
    );
    let inherited = inherit_credentials_from_existing_server(&request, &parent);
    let merged = inherited.credentials.expect("the parent fills the rest");
    let fields = fields_of(&merged);
    assert_eq!(
        fields.get("upstream_resource"),
        Some(&JsonValue::String(
            "https://payload.example.test".to_owned()
        ))
    );
    assert_eq!(
        fields.get("upstream_token_header"),
        Some(&JsonValue::String("X-Payload-Token".to_owned()))
    );
    assert_eq!(
        fields.get("auth_value"),
        Some(&JsonValue::String("parent-auth-token".to_owned()))
    );
}

#[test]
fn a_supplied_credential_skips_inheritance() {
    let parent = registry(&[], false, vec![server(PARENT_SERVER)]);
    let request = payload(r#"{"server_id": "srv-1", "credentials": {"auth_value": "user-token"}}"#);
    let inherited = inherit_credentials_from_existing_server(&request, &parent);
    let merged = inherited
        .credentials
        .expect("the supplied credential stays");
    let fields = fields_of(&merged);
    assert_eq!(fields, json_fields(r#"{"auth_value": "user-token"}"#));
}

#[rstest]
#[case::no_server_id(r#"{"transport": "http"}"#)]
#[case::empty_server_id(
    r#"{"server_id": "", "credentials": {"upstream_resource": "https://payload.example.test"}}"#
)]
#[case::unknown_server_id(r#"{"server_id": "srv-unknown"}"#)]
fn inheritance_needs_a_known_server_id(#[case] fixture: &str) {
    let parent = registry(&[], false, vec![server(PARENT_SERVER)]);
    let request = payload(fixture);
    let inherited = inherit_credentials_from_existing_server(&request, &parent);
    assert_eq!(request_fields(&inherited), request_fields(&request));
}

#[test]
fn empty_parent_values_are_not_inherited() {
    let empty_parent = registry(
        &[],
        false,
        vec![server(
            r#"{"server_id": "srv-1", "name": "Empty Parent", "transport": "http", "authentication_token": "", "client_id": "", "scopes": []}"#,
        )],
    );
    let request = payload(r#"{"server_id": "srv-1"}"#);
    let inherited = inherit_credentials_from_existing_server(&request, &empty_parent);
    assert!(inherited.credentials.is_none());
}

#[test]
fn empty_parent_values_are_left_out_of_a_merge() {
    let partial = registry(
        &[],
        false,
        vec![server(
            r#"{"server_id": "srv-1", "name": "Partial Parent", "transport": "http", "authentication_token": "", "client_id": "parent-client-id", "scopes": []}"#,
        )],
    );
    let request = payload(r#"{"server_id": "srv-1"}"#);
    let inherited = inherit_credentials_from_existing_server(&request, &partial);
    let merged = inherited.credentials.expect("the client id is inherited");
    assert_eq!(
        fields_of(&merged),
        json_fields(r#"{"client_id": "parent-client-id"}"#)
    );
}

// one truthy parent field alone must pass the inheritance guard on its own, so a broken pair in its chain shows up here
#[rstest]
#[case::authentication_token(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "authentication_token": "parent-auth-token"}"#,
    r#"{"auth_value": "parent-auth-token"}"#
)]
#[case::client_id(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "client_id": "parent-client-id"}"#,
    r#"{"client_id": "parent-client-id"}"#
)]
#[case::client_secret(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "client_secret": "parent-client-secret"}"#,
    r#"{"client_secret": "parent-client-secret"}"#
)]
#[case::scopes(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "scopes": ["read", "write"]}"#,
    r#"{"scopes": ["read", "write"]}"#
)]
#[case::aws_access_key_id(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "aws_access_key_id": "parent-access-key"}"#,
    r#"{"aws_access_key_id": "parent-access-key"}"#
)]
#[case::aws_secret_access_key(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "aws_secret_access_key": "parent-secret-key"}"#,
    r#"{"aws_secret_access_key": "parent-secret-key"}"#
)]
#[case::aws_session_token(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "aws_session_token": "parent-session-token"}"#,
    r#"{"aws_session_token": "parent-session-token"}"#
)]
#[case::aws_region_name(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "aws_region_name": "us-east-1"}"#,
    r#"{"aws_region_name": "us-east-1"}"#
)]
#[case::aws_service_name(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "aws_service_name": "bedrock-agentcore"}"#,
    r#"{"aws_service_name": "bedrock-agentcore"}"#
)]
#[case::upstream_resource(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "upstream_resource": "https://resource.example.test"}"#,
    r#"{"upstream_resource": "https://resource.example.test"}"#
)]
#[case::upstream_token_header(
    r#"{"server_id": "srv-1", "name": "Single", "transport": "http", "upstream_token_header": "X-Upstream-Token"}"#,
    r#"{"upstream_token_header": "X-Upstream-Token"}"#
)]
fn inherits_one_truthy_parent_field(#[case] parent: &str, #[case] expected: &str) {
    let parent_registry = registry(&[], false, vec![server(parent)]);
    let request = payload(r#"{"server_id": "srv-1"}"#);
    let inherited = inherit_credentials_from_existing_server(&request, &parent_registry);
    let merged = inherited
        .credentials
        .expect("the one parent field is inherited");
    assert_eq!(fields_of(&merged), json_fields(expected));
}

#[test]
fn falsy_payload_fields_win_over_inherited() {
    let parent = registry(&[], false, vec![server(PARENT_SERVER)]);
    let request = payload(
        r#"{"server_id": "srv-1", "credentials": {"client_id": null, "scopes": [], "client_secret": "", "aws_role_name": "", "aws_session_name": null}}"#,
    );
    let inherited = inherit_credentials_from_existing_server(&request, &parent);
    let merged = inherited.credentials.expect("the parent fills the rest");
    let fields = fields_of(&merged);
    assert_eq!(fields.get("client_id"), Some(&JsonValue::Null));
    assert_eq!(fields.get("scopes"), Some(&JsonValue::Array(Vec::new())));
    assert_eq!(
        fields.get("client_secret"),
        Some(&JsonValue::String(String::new()))
    );
    assert_eq!(
        fields.get("aws_role_name"),
        Some(&JsonValue::String(String::new()))
    );
    assert_eq!(fields.get("aws_session_name"), Some(&JsonValue::Null));
    assert_eq!(
        fields.get("aws_region_name"),
        Some(&JsonValue::String("us-east-1".to_owned()))
    );
}

#[test]
fn non_admin_view_strips_secret_fields() {
    let view = non_admin(FULL_ROW);
    assert_shared_stripped(&view);
    assert!(view.spec_path.is_none());
}

#[test]
fn non_admin_view_keeps_discovery_fields() {
    let view = non_admin(FULL_ROW);
    assert_eq!(view.server_name.as_deref(), Some("Test Server"));
    assert_eq!(view.alias.as_deref(), Some("test-alias"));
    assert_eq!(view.description.as_deref(), Some("test description"));
    assert_eq!(view.transport, MCPTransport::Http);
    assert_eq!(view.auth_type, Some(MCPAuth::BearerToken));
    assert_eq!(view.instructions.as_deref(), Some("test instructions"));
    assert_eq!(
        view.teams,
        vec![BTreeMap::from([(
            "team_id".to_owned(),
            Some("team-1".to_owned())
        )])]
    );
    assert_eq!(view.mcp_access_groups, ["group-1"]);
    assert_eq!(view.allowed_tools, ["tool-1"]);
    assert_eq!(
        view.health_check_error.as_deref(),
        Some("test health failure")
    );
    assert!(view.last_health_check.is_some());
    assert_eq!(view.created_by.as_deref(), Some("test-creator"));
    assert_eq!(view.updated_by.as_deref(), Some("test-updater"));
    assert_eq!(view.status, Some(McpStatus::Healthy));
    assert!(view.tool_name_to_display_name.is_some());
    assert!(view.tool_name_to_description.is_some());
}

#[test]
fn non_admin_view_keeps_the_redacted_info() {
    let info = info_fields(&non_admin(FULL_ROW));
    assert_eq!(
        info.get("owner"),
        Some(&JsonValue::String("test-owner".to_owned()))
    );
    assert_eq!(
        info.get("protocol_version"),
        Some(&JsonValue::String("2025-06-18".to_owned()))
    );
    assert_eq!(info.get("is_public"), Some(&JsonValue::Bool(false)));
}

#[test]
fn virtual_key_view_strips_secret_fields() {
    assert_shared_stripped(&virtual_key(FULL_ROW));
}

#[test]
fn virtual_key_view_keeps_spec_path() {
    let view = virtual_key(FULL_ROW);
    assert_eq!(
        view.spec_path.as_deref(),
        Some("https://mcp.example.test/openapi.json?key=test-spec-secret")
    );
}

#[test]
fn virtual_key_view_strips_access_metadata() {
    let view = virtual_key(FULL_ROW);
    assert!(view.allowed_tools.is_empty());
    assert!(view.mcp_access_groups.is_empty());
    assert!(view.teams.is_empty());
    assert!(view.health_check_error.is_none());
    assert!(view.last_health_check.is_none());
    assert!(view.created_by.is_none());
    assert!(view.updated_by.is_none());
    assert!(view.created_at.is_none());
    assert!(view.updated_at.is_none());
}

#[test]
fn virtual_key_view_keeps_safe_fields() {
    let view = virtual_key(FULL_ROW);
    assert_eq!(view.server_name.as_deref(), Some("Test Server"));
    assert_eq!(view.alias.as_deref(), Some("test-alias"));
    assert_eq!(view.description.as_deref(), Some("test description"));
    assert_eq!(view.transport, MCPTransport::Http);
    assert_eq!(view.auth_type, Some(MCPAuth::BearerToken));
    assert_eq!(view.instructions.as_deref(), Some("test instructions"));
    assert_eq!(view.status, Some(McpStatus::Healthy));
    assert!(view.tool_name_to_display_name.is_some());
}

#[test]
fn virtual_key_view_reduces_info_to_publicity() {
    let stored = r#"{"server_id": "srv-1", "transport": "http", "mcp_info": {"is_public": false, "is_public_explicit": false, "owner": "test-owner"}}"#;
    let published = ids(&["srv-1"]);
    let built = registry(
        &published,
        false,
        vec![server(
            r#"{"server_id": "srv-1", "name": "Test Server", "transport": "http"}"#,
        )],
    );
    let view = sanitize_mcp_server_for_virtual_key(&row(stored), &built, &published).unwrap();
    let info = info_fields(&view);
    assert_eq!(info.len(), 2);
    assert_eq!(info.get("is_public"), Some(&JsonValue::Bool(true)));
    assert_eq!(info.get("is_public_explicit"), Some(&JsonValue::Bool(true)));
}

#[test]
fn virtual_key_view_clears_publicity_for_an_unlisted_server() {
    let info = info_fields(&virtual_key(FULL_ROW));
    assert_eq!(info.len(), 2);
    assert_eq!(info.get("is_public"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        info.get("is_public_explicit"),
        Some(&JsonValue::Bool(false))
    );
}

#[test]
fn reduce_publicity_keeps_only_true_flags() {
    let mut stored = row(
        r#"{"server_id": "srv-1", "transport": "http", "mcp_info": {"is_public": "true", "protocol_version": "2025-06-18"}}"#,
    );
    reduce_publicity(&mut stored).unwrap();
    let info = info_fields(&stored);
    assert_eq!(info.len(), 2);
    assert_eq!(info.get("is_public"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        info.get("is_public_explicit"),
        Some(&JsonValue::Bool(false))
    );
    let mut absent = row(r#"{"server_id": "srv-2", "transport": "http"}"#);
    reduce_publicity(&mut absent).unwrap();
    assert_eq!(info_fields(&absent).len(), 2);
}

#[test]
fn sanitizers_drop_admin_config_credentials() {
    let stored = r#"{"server_id": "srv-1", "transport": "http", "credentials": {"upstream_resource": "https://resource.example.test", "scopes": ["read"], "auth_value": "test-secret"}}"#;
    assert!(non_admin(stored).credentials.is_none());
    assert!(virtual_key(stored).credentials.is_none());
}

#[test]
fn redact_list_keeps_order_and_admin_config() {
    let servers = vec![
        row(
            r#"{"server_id": "srv-1", "transport": "http", "credentials": {"auth_value": "secret-1", "upstream_resource": "https://resource.example.test"}}"#,
        ),
        row(r#"{"server_id": "srv-2", "transport": "http"}"#),
        row(
            r#"{"server_id": "srv-3", "transport": "http", "credentials": {"client_secret": "secret-3"}}"#,
        ),
    ];
    let redacted =
        redact_mcp_credentials_list(&servers, &registry(&[], false, Vec::new()), &[]).unwrap();
    assert_eq!(redacted.len(), 3);
    assert_eq!(redacted[0].server_id, "srv-1");
    assert_eq!(redacted[1].server_id, "srv-2");
    assert_eq!(redacted[2].server_id, "srv-3");
    let kept = redacted[0]
        .credentials
        .as_ref()
        .expect("admin config stays");
    assert_eq!(
        kept.upstream_resource,
        Some(Some("https://resource.example.test".to_owned()))
    );
    assert!(kept.auth_value.is_none());
    assert!(redacted[1].credentials.is_none());
    assert!(redacted[2].credentials.is_none());
}

#[test]
fn non_admin_list_applies_to_every_server() {
    let servers = vec![
        row(FULL_ROW),
        row(r#"{"server_id": "srv-2", "transport": "sse", "url": "https://mcp.example.test/sse"}"#),
    ];
    let sanitized =
        sanitize_mcp_server_list_for_non_admin(&servers, &registry(&[], false, Vec::new()), &[])
            .unwrap();
    assert_eq!(sanitized.len(), 2);
    assert_shared_stripped(&sanitized[0]);
    assert!(sanitized[0].spec_path.is_none());
    assert_shared_stripped(&sanitized[1]);
    assert!(sanitized[1].spec_path.is_none());
    assert_eq!(sanitized[1].server_id, "srv-2");
}

#[test]
fn virtual_key_list_applies_to_every_server() {
    let servers = vec![
        row(FULL_ROW),
        row(
            r#"{"server_id": "srv-2", "transport": "sse", "spec_path": "https://spec.example.test/openapi.json"}"#,
        ),
    ];
    let sanitized =
        sanitize_mcp_server_list_for_virtual_key(&servers, &registry(&[], false, Vec::new()), &[])
            .unwrap();
    assert_eq!(sanitized[0].server_id, "srv-1");
    assert_eq!(sanitized[1].server_id, "srv-2");
    assert_shared_stripped(&sanitized[0]);
    assert!(sanitized[0].allowed_tools.is_empty());
    assert!(sanitized[1].allowed_tools.is_empty());
    assert_eq!(
        sanitized[1].spec_path.as_deref(),
        Some("https://spec.example.test/openapi.json")
    );
}

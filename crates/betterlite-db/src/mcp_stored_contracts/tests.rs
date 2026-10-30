#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use super::{
    JsonValue, MAX_STORED_BYTES, MAX_STORED_DEPTH, MCPAuth, MCPAuthType, MCPCredentials,
    MCPTokenEndpointAuthMethod, MCPTransport, PinnedMCPTool, decode,
    decode::decode_mcp_stored_contract, pinned::parse_pinned_tools,
    pinned::validate_mcp_protocol_metadata,
};

fn wire(source: &str) -> JsonValue {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::sse("sse")]
#[case::http("http")]
#[case::stdio("stdio")]
fn each_transport(#[case] value: &str) {
    let encoded = serde_json::to_string(value).unwrap();
    let parsed: MCPTransport = serde_json::from_str(&encoded).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), encoded);
}

#[rstest]
#[case::none("none")]
#[case::api_key("api_key")]
#[case::bearer_token("bearer_token")]
#[case::basic("basic")]
#[case::authorization("authorization")]
#[case::oauth2("oauth2")]
#[case::aws_sigv4("aws_sigv4")]
#[case::token("token")]
#[case::oauth2_token_exchange("oauth2_token_exchange")]
#[case::oauth2_id_jag("oauth2_id_jag")]
#[case::true_passthrough("true_passthrough")]
#[case::oauth_delegate("oauth_delegate")]
fn each_auth_type(#[case] value: &str) {
    let encoded = serde_json::to_string(value).unwrap();
    let parsed: MCPAuth = serde_json::from_str(&encoded).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), encoded);
}

#[test]
fn nullable_auth() {
    assert_eq!(serde_json::from_str::<MCPAuthType>("null").unwrap(), None);
}

#[rstest]
#[case::client_secret_basic("client_secret_basic")]
#[case::client_secret_post("client_secret_post")]
fn each_token_endpoint_method(#[case] value: &str) {
    let encoded = serde_json::to_string(value).unwrap();
    let parsed: MCPTokenEndpointAuthMethod = serde_json::from_str(&encoded).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), encoded);
}

#[test]
fn all_fields_optional() {
    let credentials: MCPCredentials = serde_json::from_str("{}").unwrap();
    assert_eq!(serde_json::to_string(&credentials).unwrap(), "{}");
}
#[test]
fn explicit_null_preserved() {
    assert!(matches!(
        serde_json::from_str::<MCPCredentials>(r#"{"auth_value":null}"#)
            .unwrap()
            .auth_value,
        Some(None)
    ));
}

#[test]
fn credentials_all_fields() {
    let source = r#"{"auth_value":"secret", "client_id":"id", "client_secret":"client-secret", "scopes":["a"], "aws_access_key_id":"ak", "aws_secret_access_key":"sk", "aws_session_token":"st", "aws_region_name":"r", "aws_service_name":"s", "aws_role_name":"role", "aws_session_name":"session", "audience":"aud", "token_exchange_endpoint":"endpoint", "id_jag_resource_token_endpoint":"jag", "id_jag_resource":"resource", "upstream_resource":"upstream", "upstream_token_header":"x-token", "client_private_key":"pem", "client_private_key_id":"kid", "client_assertion_signing_alg":"RS256", "token_endpoint_auth_method":"client_secret_post", "redirect_uris":["https://a"], "token_exchange_profile":"rfc8693", "subject_token_type":"custom"}"#;
    let credentials: MCPCredentials = serde_json::from_str(source).unwrap();
    assert_eq!(
        wire(&serde_json::to_string(&credentials).unwrap()),
        wire(source)
    );
    assert_eq!(
        credentials.auth_value.unwrap().unwrap().0.expose_secret(),
        "secret"
    );
}

#[test]
fn list_fields() {
    assert_eq!(
        serde_json::from_str::<MCPCredentials>(r#"{"scopes":["a"],"redirect_uris":["b"]}"#)
            .unwrap()
            .scopes
            .unwrap()
            .unwrap(),
        ["a"]
    );
}
#[test]
fn unknown_credential_keys() {
    assert!(
        serde_json::from_str::<MCPCredentials>(r#"{"future":true}"#)
            .unwrap()
            .client_id
            .is_none()
    );
}
#[test]
fn docstring_defaults_not_materialized() {
    assert!(
        serde_json::from_str::<MCPCredentials>("{}")
            .unwrap()
            .client_assertion_signing_alg
            .is_none()
    );
}
#[test]
fn secret_debug_redaction() {
    assert!(
        !format!(
            "{:?}",
            serde_json::from_str::<MCPCredentials>(r#"{"auth_value":"do-not-log"}"#).unwrap()
        )
        .contains("do-not-log")
    );
}

#[test]
fn default_description_schema() {
    let tool: PinnedMCPTool = serde_json::from_str("{}").unwrap();
    assert_eq!(tool.as_ref().description, "");
    assert!(tool.as_ref().input_schema.is_empty());
}
#[test]
fn independent_schema_defaults() {
    let first: PinnedMCPTool = serde_json::from_str("{}").unwrap();
    let mut second: PinnedMCPTool = serde_json::from_str("{}").unwrap();
    second
        .0
        .input_schema
        .insert("x".to_owned(), JsonValue::Null);
    assert!(first.as_ref().input_schema.is_empty());
}
#[rstest]
#[case::null("null")]
#[case::boolean_false("false")]
#[case::zero("0")]
#[case::empty_string(r#""""#)]
#[case::empty_object("{}")]
#[case::empty_array("[]")]
#[case::decimal_zero("0.0")]
#[case::negative_zero("-0.0")]
fn falsey_raw_values(#[case] value: &str) {
    assert!(parse_pinned_tools(wire(value)).unwrap().is_none());
}
#[rstest]
#[case::null("null")]
#[case::boolean_false("false")]
#[case::zero("0")]
#[case::empty_string(r#""""#)]
#[case::empty_object("{}")]
#[case::empty_array("[]")]
#[case::decimal_zero("0.0")]
#[case::negative_zero("-0.0")]
fn falsey_json_values(#[case] value: &str) {
    assert!(
        parse_pinned_tools(JsonValue::String(value.to_owned()))
            .unwrap()
            .is_none()
    );
}
#[test]
fn stored_json_tools() {
    assert_eq!(
        parse_pinned_tools(JsonValue::String(
            r#"{"weather":{"description":"forecast"}}"#.to_owned()
        ))
        .unwrap()
        .unwrap()["weather"]
            .as_ref()
            .description,
        "forecast"
    );
}
#[test]
fn nested_schema_preserved() {
    let tools = parse_pinned_tools(wire(
        r#"{"tool":{"input_schema":{"properties":{"x":{"type":"string"}}}}}"#,
    ))
    .unwrap()
    .unwrap();
    assert!(
        serde_json::to_string(&tools)
            .unwrap()
            .contains("\"type\":\"string\"")
    );
}
#[test]
fn whitespace_json_error() {
    parse_pinned_tools(wire(r#""  ""#)).unwrap_err();
}
#[test]
fn nonempty_list_rejected() {
    parse_pinned_tools(wire("[\"tool\"]")).unwrap_err();
}
#[test]
fn tool_extra_field_rejected() {
    parse_pinned_tools(wire(r#"{"tool":{"extra":true}}"#)).unwrap_err();
}
#[test]
fn invalid_tool_field_type() {
    parse_pinned_tools(wire(r#"{"tool":{"description":false}}"#)).unwrap_err();
}

#[test]
fn protocol_metadata() {
    let mut metadata = BTreeMap::new();
    metadata.insert("custom".to_owned(), wire(r#"{"x":1}"#));
    metadata.insert("protocol_version".to_owned(), wire("\"auto\""));
    validate_mcp_protocol_metadata(metadata).unwrap();
}
#[test]
fn absent_version() {
    validate_mcp_protocol_metadata(BTreeMap::new()).unwrap();
}
#[test]
fn null_version_rejected() {
    let mut metadata = BTreeMap::new();
    metadata.insert("protocol_version".to_owned(), JsonValue::Null);
    validate_mcp_protocol_metadata(metadata).unwrap_err();
}
#[test]
fn future_version_rejected() {
    let mut metadata = BTreeMap::new();
    metadata.insert("protocol_version".to_owned(), wire("\"2026-07-28\""));
    validate_mcp_protocol_metadata(metadata).unwrap_err();
}

#[test]
fn mcp_stored_limits() {
    assert!(matches!(
        decode_mcp_stored_contract(&vec![b' '; MAX_STORED_BYTES + 1]),
        Err(decode::McpStoredDecodeError::TooLarge)
    ));
    let deep = format!(
        "{}0{}",
        "[".repeat(MAX_STORED_DEPTH + 2),
        "]".repeat(MAX_STORED_DEPTH + 2)
    );
    assert!(matches!(
        decode_mcp_stored_contract(deep.as_bytes()),
        Err(decode::McpStoredDecodeError::TooDeep | decode::McpStoredDecodeError::Json(_))
    ));
}
#[rstest]
fn replays_the_mcp_stored_contract_corpus(
    #[files("fuzz/corpus/mcp_stored_contract/*")] path: PathBuf,
) {
    drop(decode_mcp_stored_contract(&fs::read(path).unwrap()));
}

#[rstest::rstest]
#[case::subject_token_type(r#"{"subject_token_type":"custom"}"#)]
#[case::profile_null(r#"{"token_exchange_profile":null}"#)]
fn credential_roundtrip_regression(#[case] input: &str) {
    let credentials: MCPCredentials = serde_json::from_str(input).unwrap();
    assert_eq!(serde_json::to_string(&credentials).unwrap(), input);
}

#[rstest]
#[case::november_2024("2024-11-05")]
#[case::march_2025("2025-03-26")]
#[case::june_2025("2025-06-18")]
#[case::november_2025("2025-11-25")]
fn each_legacy_version(#[case] version: &str) {
    let metadata = BTreeMap::from([(
        "protocol_version".to_owned(),
        JsonValue::String(version.to_owned()),
    )]);
    assert_eq!(
        validate_mcp_protocol_metadata(metadata.clone()).unwrap(),
        metadata
    );
    let protocol: super::MCPUpstreamProtocol =
        serde_json::from_str(&serde_json::to_string(version).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_string(&protocol).unwrap(),
        serde_json::to_string(version).unwrap()
    );
    let legacy: super::MCPLegacyVersion =
        serde_json::from_str(&serde_json::to_string(version).unwrap()).unwrap();
    assert!(super::MCP_LEGACY_VERSIONS.contains(&legacy));
}

#[test]
fn auto_version() {
    let protocol: super::MCPUpstreamProtocol = serde_json::from_str(r#""auto""#).unwrap();
    assert_eq!(protocol, super::MCPUpstreamProtocol::Auto);
    assert_eq!(serde_json::to_string(&protocol).unwrap(), r#""auto""#);
}

#[test]
fn custom_metadata() {
    let metadata = BTreeMap::from([(
        "custom".to_owned(),
        wire(r#"{"nested":[false,null,3,"text"]}"#),
    )]);
    assert_eq!(
        validate_mcp_protocol_metadata(metadata.clone()).unwrap(),
        metadata
    );
}

#[rstest]
#[case::byte_limit(MAX_STORED_BYTES)]
#[case::below_byte_limit(MAX_STORED_BYTES - 1)]
fn mcp_stored_byte_boundary(#[case] length: usize) {
    let mut bytes = vec![b' '; length];
    bytes[0] = b'0';
    assert_eq!(decode_mcp_stored_contract(&bytes).unwrap(), wire("0"));
}

#[rstest]
#[case::at_limit(MAX_STORED_DEPTH, true)]
#[case::over_limit(MAX_STORED_DEPTH + 1, false)]
fn mcp_stored_depth_boundary(#[case] depth: usize, #[case] accepted: bool) {
    let text = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    let result = decode_mcp_stored_contract(text.as_bytes());
    if accepted {
        assert!(result.is_ok());
    } else {
        assert!(matches!(result, Err(decode::McpStoredDecodeError::TooDeep)));
    }
}

#[rstest]
#[case::bool("true")]
#[case::number("1")]
#[case::string(r#""hello""#)]
#[case::list("[1]")]
#[case::null_description(r#"{"tool":{"description":null}}"#)]
#[case::array_schema(r#"{"tool":{"input_schema":[]}}"#)]
#[case::null_schema(r#"{"tool":{"input_schema":null}}"#)]
#[case::null_tool(r#"{"tool":null}"#)]
fn malformed_tools_rejected(#[case] input: &str) {
    parse_pinned_tools(wire(input)).unwrap_err();
}

#[rstest]
#[case::trailing(r#"{"auth_value":"never-print"} trailing"#)]
#[case::broken(r#"{"auth_value":"never-print""#)]
fn decode_errors_redact_secrets(#[case] input: &str) {
    let error = decode_mcp_stored_contract(input.as_bytes()).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("never-print"));
}

#[rstest]
#[case::auth_value("auth_value")]
#[case::client_id("client_id")]
#[case::client_secret("client_secret")]
#[case::scopes("scopes")]
#[case::aws_access_key_id("aws_access_key_id")]
#[case::aws_secret_access_key("aws_secret_access_key")]
#[case::aws_session_token("aws_session_token")]
#[case::aws_region_name("aws_region_name")]
#[case::aws_service_name("aws_service_name")]
#[case::aws_role_name("aws_role_name")]
#[case::aws_session_name("aws_session_name")]
#[case::audience("audience")]
#[case::token_exchange_endpoint("token_exchange_endpoint")]
#[case::subject_token_type("subject_token_type")]
#[case::id_jag_resource_token_endpoint("id_jag_resource_token_endpoint")]
#[case::id_jag_resource("id_jag_resource")]
#[case::upstream_resource("upstream_resource")]
#[case::upstream_token_header("upstream_token_header")]
#[case::client_private_key("client_private_key")]
#[case::client_private_key_id("client_private_key_id")]
#[case::client_assertion_signing_alg("client_assertion_signing_alg")]
#[case::token_endpoint_auth_method("token_endpoint_auth_method")]
#[case::redirect_uris("redirect_uris")]
#[case::token_exchange_profile("token_exchange_profile")]
fn each_null_preserved(#[case] field: &str) {
    let input = format!(r#"{{"{field}":null}}"#);
    let credentials: MCPCredentials = serde_json::from_str(&input).unwrap();
    assert_eq!(serde_json::to_string(&credentials).unwrap(), input);
}

#[rstest]
#[case::custom(r#"{"custom":{"nested":true}}"#, true)]
#[case::null(r#"{"protocol_version":null}"#, false)]
#[case::future(r#"{"protocol_version":"2026-07-28"}"#, false)]
fn metadata_type_validates(#[case] input: &str, #[case] valid: bool) {
    let result = serde_json::from_str::<super::pinned::MCPInfo>(input);
    if valid {
        assert_eq!(serde_json::to_string(&result.unwrap()).unwrap(), input);
    } else {
        result.unwrap_err();
    }
}

#[rstest]
#[case::auth("auth_value")]
#[case::client("client_secret")]
#[case::access("aws_access_key_id")]
#[case::secret("aws_secret_access_key")]
#[case::session("aws_session_token")]
#[case::private_key("client_private_key")]
fn secret_wrapper_redacts(#[case] field: &str) {
    let input = format!(r#"{{"{field}":"never-print"}}"#);
    let credentials: MCPCredentials = serde_json::from_str(&input).unwrap();
    assert!(format!("{credentials:?}").starts_with("MCPCredentials"));
    assert!(!format!("{credentials:?}").contains("never-print"));
    assert_eq!(serde_json::to_string(&credentials).unwrap(), input);
    let secret: super::secret::StoredSecret = serde_json::from_str(r#""never-print""#).unwrap();
    assert!(!format!("{secret:?}").contains("never-print"));
}

#[rstest]
#[case::at_limit(MAX_STORED_DEPTH, true)]
#[case::over_limit(MAX_STORED_DEPTH + 1, false)]
fn mcp_stored_object_depth(#[case] depth: usize, #[case] accepted: bool) {
    let text = format!("{}0{}", r#"{"key":"#.repeat(depth), "}".repeat(depth));
    let result = decode_mcp_stored_contract(text.as_bytes());
    if accepted {
        assert!(result.is_ok());
    } else {
        assert!(matches!(result, Err(decode::McpStoredDecodeError::TooDeep)));
    }
}

#[test]
fn decoded_stored_debug_redacts() {
    let decoded = decode_mcp_stored_contract(br#"{"auth_value":"never-print"}"#).unwrap();
    assert_eq!(format!("{decoded:?}"), "[REDACTED]");
}

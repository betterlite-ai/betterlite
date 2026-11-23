#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_mcp::client::sdk_transport::error::SdkError;
use betterlite_mcp::client::sessions::error::{McpFailure, SessionConfigError, SessionError};
use betterlite_mcp::client::sessions::negotiate::ProtocolVersion;
use betterlite_mcp::client::sessions::transport::TransportType;
use betterlite_mcp::client::tools::result::error_text_result;
use betterlite_mcp::credentials::{AuthValue, McpAuthType};
use betterlite_mcp::tools::pagination::{LoadedTool, Page, ToolError, ToolFormat};
use betterlite_mcp::tools::schemas::McpTool;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;

use super::payload::{McpErrorKind, NativeMcpErrorPayload, status_payload};
use super::*;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn resolved(text: &str) -> ResolvedSettings {
    let wire: ClientWire = parse(text).unwrap();
    resolve_settings(wire, &BTreeMap::new()).unwrap()
}

fn resolved_in(text: &str, pairs: &[(&str, &str)]) -> ResolvedSettings {
    let wire: ClientWire = parse(text).unwrap();
    resolve_settings(wire, &environment(pairs)).unwrap()
}

fn refused(text: &str) -> NativeMcpErrorPayload {
    let wire: ClientWire = parse(text).unwrap();
    match resolve_settings(wire, &BTreeMap::new()) {
        Ok(_settings) => panic!("the settings text should have been refused"),
        Err(failure) => failure,
    }
}

fn wire(text: &str) -> ClientWire {
    parse(text).unwrap()
}

#[test]
fn request_defaults_match_litellm() {
    let settings = resolved("{}");
    assert_eq!(settings.server_url, "");
    assert_eq!(settings.transport_type, TransportType::Http);
    assert_eq!(settings.auth_type, McpAuthType::None);
    assert!(settings.auth_value.is_none());
    assert_eq!(settings.timeout, Duration::from_secs(60));
    assert_eq!(settings.listing_timeout, Duration::from_secs(30));
    assert_eq!(settings.npm_cache_dir, "/tmp/.npm_mcp_cache");
    assert_eq!(settings.protocol_version, ProtocolVersion::Auto);
    assert!(settings.ssl_verify.is_none());
    assert!(settings.aws.is_none());
}

#[test]
fn request_server_url_and_headers_carry_over() {
    let text = r#"{"server_url":"https://mcp.test/x","auth_header_name":"X-Key","extra_headers":{"a":"b"}}"#;
    let settings = resolved(text);
    assert_eq!(settings.server_url, "https://mcp.test/x");
    assert_eq!(settings.auth_header_name.as_deref(), Some("X-Key"));
    assert_eq!(
        settings.extra_headers.get("a").map(String::as_str),
        Some("b")
    );
}

#[test]
fn request_transport_names_match_litellm() {
    assert_eq!(
        wire(r#"{"transport_type":"sse"}"#).transport_type,
        Some(TransportType::Sse)
    );
    assert_eq!(
        wire(r#"{"transport_type":"stdio"}"#).transport_type,
        Some(TransportType::Stdio)
    );
    assert_eq!(
        wire(r#"{"transport_type":"http"}"#).transport_type,
        Some(TransportType::Http)
    );
}

#[test]
fn request_stdio_config_is_carried() {
    let text = r#"{"transport_type":"stdio","stdio_config":{"command":"npx","args":["-y","server"],"env":{"A":"1"}}}"#;
    let settings = resolved(text);
    let stdio = settings.stdio.unwrap();
    assert_eq!(stdio.command, "npx");
    assert_eq!(stdio.args, vec!["-y".to_owned(), "server".to_owned()]);
    assert_eq!(
        stdio
            .env
            .as_ref()
            .and_then(|env| env.get("A"))
            .map(String::as_str),
        Some("1")
    );
}

#[test]
fn request_timeout_comes_from_the_environment() {
    let settings = resolved_in("{}", &[("LITELLM_MCP_CLIENT_TIMEOUT", "12.5")]);
    assert_eq!(settings.timeout, Duration::from_millis(12_500));
}

#[test]
fn request_listing_timeout_comes_from_the_environment() {
    let settings = resolved_in("{}", &[("LITELLM_MCP_TOOL_LISTING_TIMEOUT", "7")]);
    assert_eq!(settings.listing_timeout, Duration::from_secs(7));
}

#[test]
fn request_empty_variable_keeps_the_default() {
    let settings = resolved_in(
        "{}",
        &[
            ("LITELLM_MCP_CLIENT_TIMEOUT", ""),
            ("LITELLM_MCP_TOOL_LISTING_TIMEOUT", "  "),
        ],
    );
    assert_eq!(settings.timeout, Duration::from_secs(60));
    assert_eq!(settings.listing_timeout, Duration::from_secs(30));
}

#[test]
fn request_invalid_environment_timeout_is_a_value_error() {
    let wire: ClientWire = parse("{}").unwrap();
    let failure = match resolve_settings(
        wire,
        &environment(&[("LITELLM_MCP_CLIENT_TIMEOUT", "soon")]),
    ) {
        Ok(_settings) => panic!("an unparsable timeout should have been refused"),
        Err(failure) => failure,
    };
    assert_eq!(failure.kind, McpErrorKind::ValueError);
    assert!(failure.message.contains("LITELLM_MCP_CLIENT_TIMEOUT"));
}

#[test]
fn request_explicit_timeout_beats_the_environment() {
    let settings = resolved_in(
        r#"{"timeout":2.0}"#,
        &[("LITELLM_MCP_CLIENT_TIMEOUT", "99")],
    );
    assert_eq!(settings.timeout, Duration::from_secs(2));
}

#[test]
fn request_negative_timeout_is_a_value_error() {
    let failure = refused(r#"{"timeout":-1.0}"#);
    assert_eq!(failure.kind, McpErrorKind::ValueError);
    assert!(failure.message.contains("timeout"));
}

#[rstest]
#[case::none("none", McpAuthType::None)]
#[case::api_key("api_key", McpAuthType::ApiKey)]
#[case::bearer_token("bearer_token", McpAuthType::BearerToken)]
#[case::basic("basic", McpAuthType::Basic)]
#[case::authorization("authorization", McpAuthType::Authorization)]
#[case::oauth2("oauth2", McpAuthType::Oauth2)]
#[case::token("token", McpAuthType::Token)]
#[case::token_exchange("oauth2_token_exchange", McpAuthType::Oauth2TokenExchange)]
fn request_auth_names_match_litellm(#[case] name: &str, #[case] expected: McpAuthType) {
    let text = format!(r#"{{"auth_type":"{name}"}}"#);
    assert_eq!(resolved(&text).auth_type, expected, "{name}");
}

#[rstest]
#[case::id_jag("oauth2_id_jag")]
#[case::passthrough("true_passthrough")]
#[case::delegate("oauth_delegate")]
fn request_unsupported_auth_names_are_refused(#[case] name: &str) {
    let failure = refused(&format!(r#"{{"auth_type":"{name}"}}"#));
    assert_eq!(failure.kind, McpErrorKind::ValueError);
    assert!(failure.message.contains(name), "{name}");
}

#[test]
fn request_text_auth_value_is_kept_as_given() {
    let settings = resolved(r#"{"auth_type":"basic","auth_value":"user:pass"}"#);
    match settings.auth_value.unwrap() {
        AuthValue::Text(secret) => assert_eq!(secret.expose_secret(), "user:pass"),
        AuthValue::Headers(_) => panic!("a text auth value stays text"),
    }
}

#[test]
fn request_header_auth_value_is_kept_as_given() {
    let settings = resolved(r#"{"auth_value":{"X-Api-Key":"secret"}}"#);
    match settings.auth_value.unwrap() {
        AuthValue::Headers(headers) => {
            assert_eq!(headers.get("X-Api-Key").unwrap().expose_secret(), "secret");
        }
        AuthValue::Text(_) => panic!("a map auth value stays a map"),
    }
}

#[test]
fn request_aws_settings_come_from_the_auth_value_map() {
    let text = r#"{"auth_type":"aws_sigv4","auth_value":{"aws_access_key_id":"AKIA","aws_secret_access_key":"shh","aws_region_name":"eu-west-1","aws_service_name":"bedrock-agentcore"}}"#;
    let settings = resolved(text);
    let aws = settings.aws.unwrap();
    assert_eq!(
        aws.aws_access_key_id
            .as_ref()
            .map(|value| value.expose_secret()),
        Some("AKIA")
    );
    assert_eq!(aws.aws_region_name.as_deref(), Some("eu-west-1"));
    assert_eq!(aws.aws_service_name.as_deref(), Some("bedrock-agentcore"));
    assert!(settings.auth_value.is_none());
    assert_eq!(settings.auth_type, McpAuthType::None);
}

#[test]
fn request_aws_text_auth_value_is_refused() {
    let failure = refused(r#"{"auth_type":"aws_sigv4","auth_value":"AKIA:shh"}"#);
    assert_eq!(failure.kind, McpErrorKind::ValueError);
    assert!(failure.message.contains("aws_sigv4"));
}

#[test]
fn request_protocol_versions_match_litellm() {
    assert_eq!(
        resolved(r#"{"protocol_version":"auto"}"#).protocol_version,
        ProtocolVersion::Auto
    );
    let legacy = resolved(r#"{"protocol_version":"2025-03-26"}"#).protocol_version;
    assert_ne!(legacy, ProtocolVersion::Auto);
    let failure = refused(r#"{"protocol_version":"2020-01-01"}"#);
    assert_eq!(failure.kind, McpErrorKind::ValueError);
}

#[test]
fn request_ssl_verify_accepts_a_bool_or_a_path() {
    assert_eq!(
        resolved(r#"{"ssl_verify":false}"#).ssl_verify,
        Some(VerifySetting::Bool(false))
    );
    assert_eq!(
        resolved(r#"{"ssl_verify":"/etc/ca.pem"}"#).ssl_verify,
        Some(VerifySetting::Path("/etc/ca.pem".to_owned()))
    );
}

#[test]
fn request_npm_cache_dir_comes_from_the_environment() {
    let settings = resolved_in("{}", &[("MCP_NPM_CACHE_DIR", "/var/cache/npm")]);
    assert_eq!(settings.npm_cache_dir, "/var/cache/npm");
    let empty = resolved_in("{}", &[("MCP_NPM_CACHE_DIR", "")]);
    assert_eq!(empty.npm_cache_dir, "/tmp/.npm_mcp_cache");
}

#[test]
fn request_call_tool_defaults() {
    let call: CallToolWire = parse("{}").unwrap();
    assert_eq!(call.name, "");
    assert!(call.arguments.is_none());
    assert!(!call.allow_input_required);
    assert!(!call.raise_on_error);
    let request = call_request(&call);
    assert_eq!(request.name, "");
    assert!(request.arguments.is_none());
}

#[test]
fn request_call_tool_carries_arguments_and_flags() {
    let call: CallToolWire =
        parse(r#"{"name":"search","arguments":{"q":"x"},"allow_input_required":true,"raise_on_error":true}"#)
            .unwrap();
    assert!(call.allow_input_required);
    assert!(call.raise_on_error);
    let request = call_request(&call);
    assert_eq!(request.name, "search");
    assert_eq!(
        request.arguments.unwrap().get("q"),
        Some(&Json::Text("x".to_owned()))
    );
}

#[test]
fn request_get_prompt_carries_named_arguments() {
    let prompt: GetPromptWire = parse(r#"{"name":"greet","arguments":{"who":"world"}}"#).unwrap();
    let params = prompt_params(prompt);
    assert_eq!(params.name, "greet");
    let arguments = params.arguments.unwrap();
    assert_eq!(arguments.get("who").map(String::as_str), Some("world"));
}

#[test]
fn request_tool_format_names_match_litellm() {
    assert_eq!(tool_format("mcp").unwrap(), ToolFormat::Mcp);
    assert_eq!(tool_format("openai").unwrap(), ToolFormat::Openai);
    let failure = tool_format("anthropic").unwrap_err();
    assert_eq!(failure.kind, McpErrorKind::ValueError);
    assert!(failure.message.contains("openai"));
}

#[test]
fn request_listing_deadline_is_null_or_seconds() {
    assert!(deadline(None).unwrap().is_none());
    assert_eq!(
        deadline(Some(2.5)).unwrap(),
        Some(Duration::from_millis(2_500))
    );
    assert!(deadline(Some(-1.0)).is_err());
}

#[test]
fn request_malformed_text_is_an_error_not_a_panic() {
    let failure = parse::<ClientWire>("{").unwrap_err();
    assert_eq!(failure.kind, McpErrorKind::ValueError);
    let wrong = parse::<CallToolWire>(r#"{"name":5}"#).unwrap_err();
    assert_eq!(wrong.kind, McpErrorKind::ValueError);
}

#[test]
fn result_page_keeps_the_cursor_name() {
    let page = Page {
        tools: vec![McpTool {
            name: "search".to_owned(),
            description: Some("finds".to_owned()),
            input_schema: None,
            extra: Object::new(),
        }],
        next_cursor: Some("page2".to_owned()),
    };
    let text = encode(&page).unwrap();
    assert!(text.contains("\"nextCursor\":\"page2\""));
}

#[test]
fn result_tool_keeps_the_wire_schema_name() {
    let mut schema = Object::new();
    schema.insert("type".to_owned(), Json::Text("object".to_owned()));
    let tool = McpTool {
        name: "search".to_owned(),
        description: None,
        input_schema: Some(schema),
        extra: Object::new(),
    };
    let text = encode(&tool).unwrap();
    assert!(text.contains("\"inputSchema\""));
}

#[test]
fn result_error_text_keeps_the_flag_and_text() {
    let result = error_text_result("McpError", "boom");
    let text = encode(&result).unwrap();
    assert!(text.contains("\"isError\":true"));
    assert!(text.contains("McpError: boom"));
}

#[test]
fn result_loaded_tools_are_untagged() {
    let tool = McpTool {
        name: "search".to_owned(),
        description: None,
        input_schema: None,
        extra: Object::new(),
    };
    let tools = vec![
        LoadedTool::Mcp(tool.clone()),
        LoadedTool::Openai(betterlite_mcp::tools::schemas::mcp_tool_to_openai(&tool)),
    ];
    let text = encode(&tools).unwrap();
    assert!(text.starts_with('['));
    assert!(text.contains("\"function\""));
}

#[test]
fn error_mcp_keeps_the_json_rpc_code() {
    let failure = session_payload(&SessionError::Protocol(McpFailure {
        code: -32022,
        message: "unsupported protocol version".to_owned(),
        context: None,
    }));
    assert_eq!(failure.kind, McpErrorKind::McpError);
    assert_eq!(failure.code, Some(-32022));
    let text = serde_json::to_string(&failure).unwrap();
    assert!(text.contains("\"code\":-32022"));
    assert!(text.contains("\"type\":\"McpError\""));
}

#[test]
fn error_read_timeout_becomes_a_timeout() {
    let failure = session_payload(&SessionError::ReadTimeout(
        betterlite_mcp::client::cleanup::failure::ReadTimeout {
            message: "the read timed out".to_owned(),
        },
    ));
    assert_eq!(failure.kind, McpErrorKind::TimeoutError);
    assert!(failure.message.contains("read timed out"));
}

#[test]
fn error_configuration_becomes_a_value_error() {
    let failure = session_payload(&SessionError::Configuration(
        SessionConfigError::MissingStdioConfig,
    ));
    assert_eq!(failure.kind, McpErrorKind::ValueError);
    assert!(failure.message.contains("stdio_config"));
}

#[test]
fn error_cancelled_names_the_cancellation() {
    let failure = session_payload(&SessionError::<SdkError>::Cancelled);
    assert_eq!(failure.kind, McpErrorKind::CancelledError);
}

#[test]
fn error_status_keeps_the_status_and_the_body() {
    let failure = status_payload(401, Some("denied"));
    assert_eq!(failure.kind, McpErrorKind::RuntimeError);
    assert!(failure.message.contains("401"));
    let data = failure.data.unwrap();
    match data {
        Json::Object(object) => {
            assert_eq!(
                object.get("status"),
                Some(&Json::Number(serde_json::Number::from(401_u16)))
            );
            assert_eq!(object.get("body"), Some(&Json::Text("denied".to_owned())));
        }
        other => panic!("expected an object, got {other:?}"),
    }
}

#[test]
fn error_status_without_a_capture_says_so() {
    let failure = status_payload(500, None);
    let data = failure.data.unwrap();
    match data {
        Json::Object(object) => {
            assert_eq!(
                object.get("body"),
                Some(&Json::Text("(no body captured)".to_owned()))
            );
        }
        other => panic!("expected an object, got {other:?}"),
    }
}

#[tokio::test]
async fn error_tool_timeout_becomes_a_timeout() {
    let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let failure = tool_failure(&ToolError::<SessionError<SdkError>>::Timeout(elapsed));
    assert_eq!(failure.kind, McpErrorKind::TimeoutError);
    assert!(failure.message.contains("timed out"));
}

#[test]
fn error_payload_shape_names_every_field() {
    let failure = NativeMcpErrorPayload::plain(McpErrorKind::RuntimeError, "boom".to_owned());
    let text = serde_json::to_string(&failure).unwrap();
    assert!(text.contains("\"type\":\"RuntimeError\""));
    assert!(text.contains("\"message\":\"boom\""));
    assert!(text.contains("\"code\":null"));
    assert!(text.contains("\"data\":null"));
}

#[test]
fn result_secret_text_is_not_mistaken_for_an_error() {
    let value = SecretString::from("kept");
    assert_eq!(value.expose_secret(), "kept");
    assert_eq!(encode(&Json::Null).unwrap(), "null");
}

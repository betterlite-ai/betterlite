#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_a2a::card_resolver::fetch_error::{CardFetchError, ProbeFailure};
use betterlite_a2a::client::error::ClientError;
use betterlite_a2a::client::records::compat::Role;
use betterlite_a2a::client::retry::SendError;
use betterlite_a2a::completion_bridge::dispatch::BridgeError;
use betterlite_a2a::errors::A2AError;
use betterlite_a2a::errors::retries::RetryCounts;
use betterlite_a2a::send::error::{SendCallError, SendEntryError};
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_core::a2a::exception_mapping::RecoveryError;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::{JsonRpcId, LiteLLMSendMessageResponse};
use pyo3::prelude::*;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{Call, Settings, parse, response_text};
use crate::a2a::error::A2aCallError;
use crate::a2a::payload::{ErrorPayload, entry_error, send_payload};
use crate::a2a::ports::Unavailable;

const SEND_CALL: &str = r#"{"request":{"id":"req-1","params":{"message":{"role":"user","parts":[{"kind":"text","text":"ping"}],"messageId":"m-1"}}},"api_base":"http://agent.example","litellm_params":{"model":"agent"},"agent_id":"agent-7","agent_extra_headers":{"X-Agent":"a"},"litellm_call_id":"call-1","litellm_trace_id":"trace-1","metadata":{"user_id":"u"}}"#;

fn tls() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: String::new(),
    }
}

fn written(payload: &ErrorPayload) -> AgentJson {
    let text = serde_json::to_string(payload).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn text_field(json: &AgentJson, key: &str) -> Option<String> {
    json.get(key).and_then(AgentJson::as_str).map(str::to_owned)
}

fn number_field(json: &AgentJson, key: &str) -> Option<u64> {
    json.get(key).and_then(AgentJson::as_u64)
}

fn header(headers: &Option<BTreeMap<String, String>>, name: &str) -> Option<String> {
    headers
        .as_ref()
        .and_then(|pairs| pairs.get(name).map(str::to_owned))
}

#[test]
fn request_settings_take_every_given_key() {
    let text = r#"{"base_url":"http://agent.example","timeout":3.5,"extra_headers":{"X-Test":"1"},"streaming":true,"relative_card_path":"agentCard/v1.0"}"#;
    let settings = parse::<Settings>(text).unwrap();
    assert_eq!(settings.base_url, "http://agent.example");
    assert_eq!(settings.timeout, Some(3.5));
    assert_eq!(settings.streaming, Some(true));
    assert_eq!(
        settings.relative_card_path.as_deref(),
        Some("agentCard/v1.0")
    );
    assert_eq!(
        header(&settings.extra_headers, "X-Test").as_deref(),
        Some("1")
    );
    let built = settings.into_settings(tls()).unwrap();
    assert_eq!(built.base_url, "http://agent.example");
    assert_eq!(built.timeout, Some(Duration::from_millis(3_500)));
    assert!(built.streaming);
    assert_eq!(built.relative_card_path.as_deref(), Some("agentCard/v1.0"));
    assert_eq!(built.extra_headers.len(), 1);
    assert_eq!(
        built
            .extra_headers
            .get("X-Test")
            .map(|value| value.expose_secret()),
        Some("1")
    );
}

#[test]
fn request_settings_default_every_absent_key() {
    let settings = parse::<Settings>("{}").unwrap();
    assert_eq!(settings.base_url, "");
    assert_eq!(settings.timeout, None);
    assert_eq!(settings.streaming, None);
    assert!(settings.extra_headers.is_none());
    assert!(settings.relative_card_path.is_none());
    let built = settings.into_settings(tls()).unwrap();
    assert!(!built.streaming);
    assert!(built.timeout.is_none());
    assert!(built.extra_headers.is_empty());
}

#[test]
fn request_settings_take_an_explicit_null_as_the_default() {
    let text = r#"{"base_url":"http://agent.example","timeout":null,"extra_headers":null,"streaming":null,"relative_card_path":null}"#;
    let settings = parse::<Settings>(text).unwrap();
    assert_eq!(settings.timeout, None);
    assert_eq!(settings.streaming, None);
    assert!(settings.extra_headers.is_none());
    assert!(settings.relative_card_path.is_none());
    assert!(!settings.into_settings(tls()).unwrap().streaming);
}

#[rstest]
#[case::base_url(r#"{"base_url":5}"#)]
#[case::timeout(r#"{"timeout":"3"}"#)]
#[case::streaming(r#"{"streaming":"yes"}"#)]
#[case::extra_headers(r#"{"extra_headers":{"X-Test":1}}"#)]
#[case::relative_card_path(r#"{"relative_card_path":5}"#)]
fn request_settings_reject_a_wrong_type(#[case] text: &str) {
    assert!(matches!(
        parse::<Settings>(text),
        Err(A2aCallError::Json { .. })
    ));
}

#[rstest]
#[case::zero(r#"{"base_url":"u","timeout":0.0}"#, Ok(Duration::ZERO))]
#[case::negative(r#"{"base_url":"u","timeout":-1.0}"#, Err("timeout"))]
#[case::absent(r#"{"base_url":"u"}"#, Ok(None))]
fn request_settings_resolve_the_timeout_seconds(
    #[case] text: &str,
    #[case] expected: Result<Option<Duration>, &str>,
) {
    let outcome = parse::<Settings>(text)
        .unwrap()
        .into_settings(tls())
        .map(|built| built.timeout);
    match (outcome, expected) {
        (Ok(Some(found)), Ok(_zero)) => assert_eq!(found, _zero),
        (Ok(None), Ok(_zero)) => panic!("expected the configured timeout, got none"),
        (Err(A2aCallError::Timeout { field }), Err(want)) => assert_eq!(field, want),
        (found, want) => panic!("expected {want:?}, got {found:?}"),
    }
}

#[test]
fn request_call_resolves_every_send_field() {
    let resolved = parse::<Call>(SEND_CALL).unwrap().resolve(None).unwrap();
    assert!(resolved.client.is_none());
    assert_eq!(
        resolved.request.id,
        JsonRpcId::String(String::from("req-1"))
    );
    assert_eq!(resolved.request.params.message.role, Role::User);
    assert_eq!(resolved.request.params.message.parts.len(), 1);
    assert_eq!(resolved.api_base.as_deref(), Some("http://agent.example"));
    assert_eq!(
        resolved
            .litellm_params
            .get("model")
            .and_then(AgentJson::as_str),
        Some("agent")
    );
    assert_eq!(resolved.agent_id.as_deref(), Some("agent-7"));
    assert_eq!(resolved.trace_id.as_deref(), Some("trace-1"));
    assert_eq!(
        header(&resolved.agent_extra_headers, "X-Agent").as_deref(),
        Some("a")
    );
    assert!(resolved.metadata.is_some());
}

#[test]
fn request_call_defaults_the_optional_keys() {
    let text = r#"{"request":{"id":7,"params":{"message":{"role":"user","parts":[]}}}}"#;
    let resolved = parse::<Call>(text).unwrap().resolve(None).unwrap();
    assert_eq!(serde_json::to_string(&resolved.request.id).unwrap(), "7");
    assert!(resolved.api_base.is_none());
    assert!(resolved.litellm_params.is_empty());
    assert!(resolved.agent_id.is_none());
    assert!(resolved.agent_extra_headers.is_none());
    assert!(resolved.trace_id.is_none());
    assert!(resolved.metadata.is_none());
}

#[rstest]
#[case::plain("{}", "request is required")]
#[case::bridge(
    r#"{"litellm_params":{"custom_llm_provider":"langgraph"}}"#,
    "request is required for completion bridge"
)]
#[case::empty_provider(
    r#"{"litellm_params":{"custom_llm_provider":""}}"#,
    "request is required"
)]
#[case::null_provider(
    r#"{"litellm_params":{"custom_llm_provider":null}}"#,
    "request is required"
)]
#[case::number_provider(
    r#"{"litellm_params":{"custom_llm_provider":7}}"#,
    "request is required"
)]
fn request_call_states_a_missing_request_like_litellm(#[case] text: &str, #[case] message: &str) {
    let sent = match parse::<Call>(text).unwrap().resolve(None) {
        Ok(_call) => panic!("a request-less call was sent"),
        Err(error) => error.payload(),
    };
    assert_eq!(sent.kind, "ValueError");
    assert_eq!(sent.message, message);
}

#[test]
fn request_call_rejects_malformed_text() {
    assert!(matches!(parse::<Call>("{"), Err(A2aCallError::Json { .. })));
    assert!(matches!(
        parse::<Call>(r#"{"request":{}}"#),
        Err(A2aCallError::Json { .. })
    ));
    assert!(matches!(
        parse::<Settings>("[]"),
        Err(A2aCallError::Json { .. })
    ));
}

#[test]
fn result_response_text_carries_every_send_message_key() {
    let source = r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"message","messageId":"m-1","role":"agent","parts":[{"kind":"text","text":"pong"}]},"error":null,"usage":{"total_tokens":7},"_hidden_params":{"leak":true}}"#;
    let response: LiteLLMSendMessageResponse = serde_json::from_str(source).unwrap();
    let text = response_text(&response).unwrap();
    let json: AgentJson = serde_json::from_str(&text).unwrap();
    assert_eq!(
        json.get("_hidden_params")
            .map(AgentJson::to_string)
            .as_deref(),
        Some("{}")
    );
    assert_eq!(text_field(&json, "id").as_deref(), Some("req-1"));
    assert_eq!(text_field(&json, "jsonrpc").as_deref(), Some("2.0"));
    assert_eq!(
        json.get("result")
            .and_then(|result| result.get("kind"))
            .and_then(AgentJson::as_str),
        Some("message")
    );
    assert_eq!(
        json.get("usage")
            .and_then(|usage| usage.get("total_tokens"))
            .and_then(AgentJson::as_u64),
        Some(7)
    );
    assert!(json.get("error").is_some_and(AgentJson::is_null));
}

#[test]
fn result_response_text_carries_the_hidden_params_it_holds() {
    let source = r#"{"id":"req-3","jsonrpc":"2.0","result":null,"_hidden_params":{"leak":true}}"#;
    let mut response: LiteLLMSendMessageResponse = serde_json::from_str(source).unwrap();
    response.hidden_params.insert(
        String::from("model"),
        AgentJson::String(String::from("agent")),
    );
    let json: AgentJson = serde_json::from_str(&response_text(&response).unwrap()).unwrap();
    let hidden = json.get("_hidden_params").unwrap();
    assert_eq!(
        hidden.get("model").and_then(AgentJson::as_str),
        Some("agent")
    );
    assert!(hidden.get("leak").is_none());
}

#[test]
fn result_response_text_keeps_a_json_rpc_error() {
    let source =
        r#"{"id":"req-2","jsonrpc":"2.0","result":null,"error":{"code":-32000,"message":"boom"}}"#;
    let response: LiteLLMSendMessageResponse = serde_json::from_str(source).unwrap();
    let json: AgentJson = serde_json::from_str(&response_text(&response).unwrap()).unwrap();
    assert_eq!(
        json.get("error")
            .and_then(|error| error.get("code"))
            .and_then(AgentJson::as_i64),
        Some(-32_000)
    );
    assert_eq!(
        json.get("error")
            .and_then(|error| error.get("message"))
            .and_then(AgentJson::as_str),
        Some("boom")
    );
    assert!(json.get("result").is_some_and(AgentJson::is_null));
}

#[test]
fn error_base_carries_the_message_retries_and_provider() {
    let error = A2AError::Base {
        message: String::from("boom"),
        model: Some(String::from("agent")),
        debug_info: Some(String::from("dbg")),
        retries: RetryCounts {
            retried: Some(1),
            max: Some(2),
        },
    };
    let json = written(&ErrorPayload::from_a2a(&error));
    assert_eq!(text_field(&json, "type").as_deref(), Some("A2AError"));
    assert_eq!(text_field(&json, "message").as_deref(), Some("boom"));
    assert_eq!(number_field(&json, "status_code"), Some(500));
    assert_eq!(
        text_field(&json, "llm_provider").as_deref(),
        Some("a2a_agent")
    );
    assert_eq!(text_field(&json, "model").as_deref(), Some("agent"));
    assert_eq!(
        text_field(&json, "litellm_debug_info").as_deref(),
        Some("dbg")
    );
    assert_eq!(number_field(&json, "max_retries"), Some(2));
    assert_eq!(number_field(&json, "num_retries"), Some(1));
    assert!(json.get("url").is_some_and(AgentJson::is_null));
}

#[test]
fn error_connection_carries_the_url_and_its_status() {
    let error = A2AError::Connection {
        message: String::from("down"),
        url: Some(String::from("http://agent.example/")),
        model: None,
        debug_info: None,
        retries: RetryCounts::default(),
    };
    let json = written(&ErrorPayload::from_a2a(&error));
    assert_eq!(
        text_field(&json, "type").as_deref(),
        Some("A2AConnectionError")
    );
    assert_eq!(number_field(&json, "status_code"), Some(503));
    assert_eq!(
        text_field(&json, "url").as_deref(),
        Some("http://agent.example/")
    );
    assert_eq!(
        text_field(&json, "llm_provider").as_deref(),
        Some("a2a_agent")
    );
}

#[rstest]
#[case::missing(404_u16)]
#[case::refused(502_u16)]
fn error_agent_card_keeps_its_status_code(#[case] status: u16) {
    let error = A2AError::AgentCard {
        message: String::from("no card"),
        url: Some(String::from("http://agent.example/")),
        status_code: status,
    };
    let json = written(&ErrorPayload::from_a2a(&error));
    assert_eq!(
        text_field(&json, "type").as_deref(),
        Some("A2AAgentCardError")
    );
    assert_eq!(number_field(&json, "status_code"), Some(u64::from(status)));
    assert_eq!(
        text_field(&json, "url").as_deref(),
        Some("http://agent.example/")
    );
}

#[test]
fn error_discovery_lists_every_failed_path() {
    let error = A2AError::agent_card_discovery(
        String::from("http://agent.example"),
        vec![
            ProbeFailure {
                path: String::from("/.well-known/agent-card.json"),
                reason: CardFetchError::Status { status: 404 },
            },
            ProbeFailure {
                path: String::from("/.well-known/agent.json"),
                reason: CardFetchError::Status { status: 502 },
            },
        ],
    );
    let json = written(&ErrorPayload::from_a2a(&error));
    assert_eq!(
        text_field(&json, "type").as_deref(),
        Some("A2AAgentCardDiscoveryError")
    );
    assert_eq!(
        text_field(&json, "message").as_deref(),
        Some(
            "Failed to fetch agent card from http://agent.example. Tried /.well-known/agent-card.json (HTTP 404), /.well-known/agent.json (HTTP 502)"
        )
    );
    assert_eq!(number_field(&json, "status_code"), Some(502));
    assert_eq!(
        text_field(&json, "base_url").as_deref(),
        Some("http://agent.example")
    );
    assert_eq!(
        text_field(&json, "url").as_deref(),
        Some("http://agent.example")
    );
    let failures = json.get("failures").and_then(AgentJson::as_array).unwrap();
    assert_eq!(failures.len(), 2);
    assert_eq!(
        text_field(&failures[0], "path").as_deref(),
        Some("/.well-known/agent-card.json")
    );
    assert_eq!(
        text_field(&failures[0], "error").as_deref(),
        Some("HTTP 404")
    );
    assert_eq!(
        text_field(&failures[1], "error").as_deref(),
        Some("HTTP 502")
    );
}

#[test]
fn error_discovery_with_only_missing_paths_reports_404() {
    let error = A2AError::agent_card_discovery(
        String::from("http://agent.example"),
        vec![ProbeFailure {
            path: String::from("/.well-known/agent-card.json"),
            reason: CardFetchError::Status { status: 404 },
        }],
    );
    let json = written(&ErrorPayload::from_a2a(&error));
    assert_eq!(number_field(&json, "status_code"), Some(404));
}

#[test]
fn error_localhost_url_names_the_original() {
    let original = A2AError::Connection {
        message: String::from("connection refused"),
        url: Some(String::from("http://localhost:9/")),
        model: None,
        debug_info: None,
        retries: RetryCounts::default(),
    };
    let error = A2AError::LocalhostUrl {
        localhost_url: String::from("http://localhost:9/"),
        base_url: String::from("http://agent.example"),
        model: Some(String::from("agent")),
        original_error: Some(Box::new(original)),
    };
    let json = written(&ErrorPayload::from_a2a(&error));
    assert_eq!(
        text_field(&json, "type").as_deref(),
        Some("A2ALocalhostURLError")
    );
    assert_eq!(number_field(&json, "status_code"), Some(503));
    assert_eq!(
        text_field(&json, "message").as_deref(),
        Some(
            "Agent card contains localhost/internal URL 'http://localhost:9/'. Retrying with base URL 'http://agent.example'."
        )
    );
    assert_eq!(
        text_field(&json, "url").as_deref(),
        Some("http://localhost:9/")
    );
    assert_eq!(
        text_field(&json, "localhost_url").as_deref(),
        Some("http://localhost:9/")
    );
    assert_eq!(
        text_field(&json, "base_url").as_deref(),
        Some("http://agent.example")
    );
    assert_eq!(text_field(&json, "model").as_deref(), Some("agent"));
    assert_eq!(
        text_field(&json, "original_error").as_deref(),
        Some("connection refused")
    );
}

#[test]
fn error_localhost_url_without_an_original_is_null() {
    let error = A2AError::LocalhostUrl {
        localhost_url: String::from("http://localhost:9/"),
        base_url: String::from("http://agent.example"),
        model: None,
        original_error: None,
    };
    let json = written(&ErrorPayload::from_a2a(&error));
    assert!(json.get("original_error").is_some_and(AgentJson::is_null));
}

#[test]
fn error_value_and_runtime_payloads_name_their_kind() {
    let missing = A2aCallError::MissingRequest.payload();
    assert_eq!(missing.kind, "ValueError");
    assert_eq!(missing.message, "request is required");
    assert!(missing.status_code.is_none());
    let bridge = A2aCallError::MissingBridgeRequest.payload();
    assert_eq!(bridge.kind, "ValueError");
    assert_eq!(bridge.message, "request is required for completion bridge");
    let malformed = A2aCallError::Json {
        source: serde_json::from_str::<AgentJson>("{").unwrap_err(),
    };
    assert_eq!(malformed.payload().kind, "ValueError");
    assert!(malformed.payload().message.contains("expected"));
    let encode = A2aCallError::Encode {
        source: serde_json::from_str::<AgentJson>("{").unwrap_err(),
    };
    assert_eq!(encode.payload().kind, "RuntimeError");
    let timeout = A2aCallError::Timeout { field: "timeout" }.payload();
    assert_eq!(timeout.kind, "ValueError");
    assert_eq!(
        timeout.message,
        "the a2a settings field 'timeout' is not a valid number of seconds"
    );
    let card = A2aCallError::A2A(A2AError::AgentCard {
        message: String::from("no card"),
        url: None,
        status_code: 404,
    })
    .payload();
    assert_eq!(card.kind, "A2AAgentCardError");
}

#[test]
fn error_send_payload_maps_every_send_call_error() {
    let missing = send_payload(SendCallError::MissingClient, false);
    assert_eq!(missing.kind, "ValueError");
    assert_eq!(
        missing.message,
        "Either a2a_client or api_base is required for standard A2A flow"
    );
    let not_text = send_payload(
        SendCallError::NotText {
            field: "custom_llm_provider",
        },
        false,
    );
    assert_eq!(not_text.kind, "A2AError");
    assert_eq!(not_text.status_code, Some(500));
    assert_eq!(
        not_text.message,
        "the A2A litellm_params field 'custom_llm_provider' must be a string"
    );
    let client = send_payload(
        SendCallError::Client(A2AError::Connection {
            message: String::from("down"),
            url: Some(String::from("http://agent.example/")),
            model: None,
            debug_info: None,
            retries: RetryCounts::default(),
        }),
        false,
    );
    assert_eq!(client.kind, "A2AConnectionError");
    let no_response = send_payload(SendCallError::Send(SendError::NoResponse), false);
    assert_eq!(no_response.kind, "RuntimeError");
    assert_eq!(
        no_response.message,
        "A2A send_message failed: no response received after retry attempts."
    );
    let streamed = send_payload(SendCallError::Send(SendError::NoResponse), true);
    assert_eq!(
        streamed.message,
        "A2A send_message_streaming failed: no response received after retry attempts."
    );
    let transport = send_payload(
        SendCallError::Send(SendError::Client(Box::new(ClientError::Status {
            url: String::from("http://agent.example/"),
            status: 502,
        }))),
        false,
    );
    assert_eq!(transport.kind, "A2AConnectionError");
    assert_eq!(transport.status_code, Some(503));
    assert_eq!(transport.message, "the agent answered HTTP 502");
    assert_eq!(transport.url.as_deref(), Some("http://agent.example/"));
    let unnamed = send_payload(
        SendCallError::Send(SendError::Client(Box::new(ClientError::MissingEndpoint))),
        false,
    );
    assert_eq!(unnamed.kind, "A2AConnectionError");
    assert!(unnamed.url.is_none());
    let recovery = send_payload(
        SendCallError::Send(SendError::Recovery(Box::new(
            RecoveryError::<ClientError>::MissingAgentCard,
        ))),
        false,
    );
    assert_eq!(recovery.kind, "RuntimeError");
    assert!(
        recovery
            .message
            .starts_with("cannot retry the A2A localhost URL fix")
    );
    let nested = send_payload(
        SendCallError::Send(SendError::A2A(Box::new(A2AError::Base {
            message: String::from("boom"),
            model: None,
            debug_info: None,
            retries: RetryCounts::default(),
        }))),
        false,
    );
    assert_eq!(nested.kind, "A2AError");
    assert_eq!(nested.message, "boom");
    let token = send_payload(
        SendCallError::TokenCount(u64::try_from(-1_i64).unwrap_err()),
        false,
    );
    assert_eq!(token.kind, "A2AError");
    assert_eq!(
        token.message,
        "the A2A usage token count does not fit a u64"
    );
}

#[test]
fn error_entry_error_wraps_both_sources() {
    Python::initialize();
    Python::attach(|py| {
        let call = entry_error(
            SendEntryError::<Unavailable, Unavailable>::Call(SendCallError::MissingClient),
            false,
        );
        let json: AgentJson = serde_json::from_str(&call.value(py).to_string()).unwrap();
        assert_eq!(text_field(&json, "type").as_deref(), Some("ValueError"));
        assert_eq!(
            text_field(&json, "message").as_deref(),
            Some("Either a2a_client or api_base is required for standard A2A flow")
        );
        let bridge = entry_error(
            SendEntryError::<Unavailable, Unavailable>::Bridge(BridgeError::Provider(Unavailable)),
            true,
        );
        let json: AgentJson = serde_json::from_str(&bridge.value(py).to_string()).unwrap();
        assert_eq!(text_field(&json, "type").as_deref(), Some("RuntimeError"));
        assert_eq!(
            text_field(&json, "message").as_deref(),
            Some("the A2A provider config failed")
        );
    });
}

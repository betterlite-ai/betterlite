#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{
    Agent365PayloadError, MCP_SESSION_ID_HEADER, build_evaluate_payload, resolve_conversation_id,
};

type Scenario = fn();

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn object_fields(pairs: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn test_payload_defaults() {
    let fields = object_fields(&[("litellm_call_id", text("call-1"))]);
    let payload = build_evaluate_payload(&fields, None).unwrap();
    assert_eq!(payload.tool.name, "");
    assert_eq!(payload.server_name, "litellm");
    assert_eq!(payload.conversation_id, "call-1");
    assert!(payload.arguments.is_none());
    assert!(payload.agent_id.is_none());
    assert_eq!(
        serde_json::to_string(&payload).unwrap(),
        r#"{"tool":{"name":""},"serverName":"litellm","conversationId":"call-1"}"#
    );
}

fn test_payload_arguments_and_alias() {
    let arguments = object_fields(&[("q", text("weather"))]);
    let fields = object_fields(&[
        ("mcp_tool_name", text("search")),
        ("mcp_server_name", text("srv")),
        ("mcp_arguments", GuardrailValue::Object(arguments.clone())),
        ("litellm_call_id", text("call-1")),
    ]);
    let payload = build_evaluate_payload(&fields, Some("alias-1")).unwrap();
    assert_eq!(payload.arguments, Some(arguments));
    assert_eq!(payload.agent_id.as_deref(), Some("alias-1"));
    assert_eq!(
        serde_json::to_string(&payload).unwrap(),
        r#"{"tool":{"name":"search"},"serverName":"srv","conversationId":"call-1","arguments":{"q":"weather"},"agentId":"alias-1"}"#
    );
    let empty_alias = build_evaluate_payload(&fields, Some("")).unwrap();
    assert!(empty_alias.agent_id.is_none());
}

fn test_payload_omits_non_object_arguments() {
    for omitted in [
        GuardrailValue::Text("not-an-object".to_owned()),
        GuardrailValue::Array(vec![text("x")]),
        GuardrailValue::Null,
    ] {
        let fields = object_fields(&[
            ("mcp_tool_name", text("search")),
            ("mcp_arguments", omitted),
        ]);
        let payload = build_evaluate_payload(&fields, None).unwrap();
        assert!(payload.arguments.is_none());
    }
}

fn test_payload_rejects_non_string_names() {
    let tool = object_fields(&[("mcp_tool_name", GuardrailValue::SignedInteger(5))]);
    assert!(matches!(
        build_evaluate_payload(&tool, None),
        Err(Agent365PayloadError::TextField {
            field: "mcp_tool_name"
        })
    ));
    let server = object_fields(&[("mcp_server_name", GuardrailValue::Boolean(true))]);
    assert!(matches!(
        build_evaluate_payload(&server, None),
        Err(Agent365PayloadError::TextField {
            field: "mcp_server_name"
        })
    ));
}

fn test_conversation_id_from_metadata_and_headers() {
    let logging_session = object_fields(&[
        (
            "mcp_tool_call_metadata",
            GuardrailValue::Object(object_fields(&[("mcp_session_id", text("session-1"))])),
        ),
        ("litellm_call_id", text("call-1")),
    ]);
    assert_eq!(resolve_conversation_id(&logging_session), "session-1");

    let header = object_fields(&[
        (
            "metadata",
            GuardrailValue::Object(object_fields(&[(
                "headers",
                GuardrailValue::Object(object_fields(&[(
                    "MCP-Session-ID",
                    text("header-session"),
                )])),
            )])),
        ),
        ("litellm_call_id", text("call-1")),
    ]);
    assert_eq!(resolve_conversation_id(&header), "header-session");

    let nested = object_fields(&[(
        "litellm_metadata",
        GuardrailValue::Object(object_fields(&[(
            "headers",
            GuardrailValue::Object(object_fields(&[(
                MCP_SESSION_ID_HEADER,
                text("nested-session"),
            )])),
        )])),
    )]);
    assert_eq!(resolve_conversation_id(&nested), "nested-session");
}

fn test_conversation_id_from_nested_tool_metadata() {
    let nested_tool = object_fields(&[(
        "metadata",
        GuardrailValue::Object(object_fields(&[
            (
                "mcp_tool_call_metadata",
                GuardrailValue::Object(object_fields(&[("mcp_session_id", text("nested-tool"))])),
            ),
            (
                "headers",
                GuardrailValue::Object(object_fields(&[(
                    MCP_SESSION_ID_HEADER,
                    text("header-session"),
                )])),
            ),
        ])),
    )]);
    assert_eq!(resolve_conversation_id(&nested_tool), "nested-tool");
}

fn test_conversation_id_skips_empty_sources() {
    let ignored = object_fields(&[(
        "headers",
        GuardrailValue::Object(object_fields(&[(MCP_SESSION_ID_HEADER, text("ignored"))])),
    )]);
    let empty_metadata = object_fields(&[
        ("metadata", GuardrailValue::Object(object_fields(&[]))),
        ("litellm_metadata", GuardrailValue::Object(ignored)),
        ("litellm_call_id", text("call-1")),
    ]);
    assert_eq!(resolve_conversation_id(&empty_metadata), "call-1");

    let blank = object_fields(&[(
        "headers",
        GuardrailValue::Object(object_fields(&[("mcp-session-id", text(""))])),
    )]);
    let empty_header = object_fields(&[
        ("metadata", GuardrailValue::Object(blank)),
        ("litellm_call_id", text("call-1")),
    ]);
    assert_eq!(resolve_conversation_id(&empty_header), "call-1");

    for session in [GuardrailValue::SignedInteger(5), text("")] {
        let data = object_fields(&[
            (
                "mcp_tool_call_metadata",
                GuardrailValue::Object(object_fields(&[("mcp_session_id", session)])),
            ),
            ("litellm_call_id", text("call-1")),
        ]);
        assert_eq!(resolve_conversation_id(&data), "call-1");
    }
}

fn test_conversation_id_generated() {
    let generated = resolve_conversation_id(&object_fields(&[]));
    let parsed = uuid::Uuid::parse_str(&generated).unwrap();
    assert_eq!(parsed.get_version_num(), 4);
    assert_eq!(parsed.to_string(), generated);
    assert_ne!(resolve_conversation_id(&object_fields(&[])), generated);

    let empty_call_id = object_fields(&[("litellm_call_id", text(""))]);
    let fallback = resolve_conversation_id(&empty_call_id);
    assert_eq!(
        uuid::Uuid::parse_str(&fallback).unwrap().get_version_num(),
        4
    );
}

#[rstest]
#[case::payload_defaults(test_payload_defaults)]
#[case::payload_arguments_and_alias(test_payload_arguments_and_alias)]
#[case::payload_omits_non_object_arguments(test_payload_omits_non_object_arguments)]
#[case::payload_rejects_non_string_names(test_payload_rejects_non_string_names)]
#[case::conversation_id_from_metadata_and_headers(test_conversation_id_from_metadata_and_headers)]
#[case::conversation_id_from_nested_tool_metadata(test_conversation_id_from_nested_tool_metadata)]
#[case::conversation_id_skips_empty_sources(test_conversation_id_skips_empty_sources)]
#[case::conversation_id_generated(test_conversation_id_generated)]
fn agent_365_token_behaviors(#[case] scenario: Scenario) {
    scenario();
}

#![cfg(test)]

use std::str::FromStr as _;

use rust_decimal::Decimal;

use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::blocking::{self, ReplacedResponse, TextContent};

fn content_text(json: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Shape {
        error: Option<String>,
        message: Option<String>,
    }
    serde_json::from_str::<Shape>(json).map_or_else(
        |_| json.to_owned(),
        |shape| {
            shape
                .message
                .unwrap_or_else(|| shape.error.unwrap_or_default())
        },
    )
}

#[test]
fn blocking_response_passes_dict_detail_through_as_json() {
    let detail = Json::parse(r#"{"reason": "blocked", "code": 7}"#);
    let response = blocking::build_blocking_response(&detail, "cisco", Some("my-guardrail"), None);
    let [item] = &response.mcp_tool_call_response;
    assert_eq!(item.kind, "text");
    assert_eq!(item.text, r#"{"reason":"blocked","code":7}"#);
    assert!(response.hidden_params.response_cost.is_none());
}

#[derive(serde::Deserialize)]
struct SynthesizedPayloadShape {
    error: String,
    message: String,
    provider: String,
    guardrail: Option<String>,
    surface: String,
    direction: String,
    action: String,
}

#[test]
fn blocking_response_synthesizes_payload_from_string_detail() {
    let detail = Json::parse(r#""rule 42 matched""#);
    let response = blocking::build_blocking_response(&detail, "cisco", Some("my-guardrail"), None);
    let [item] = &response.mcp_tool_call_response;
    let payload: SynthesizedPayloadShape = serde_json::from_str(&item.text).unwrap();
    assert_eq!(payload.error, "Blocked by Cisco AI Defense Guardrail");
    assert_eq!(payload.message, "rule 42 matched");
    assert_eq!(payload.provider, "cisco");
    assert_eq!(payload.guardrail.as_deref(), Some("my-guardrail"));
    assert_eq!(payload.surface, "mcp");
    assert_eq!(payload.direction, "output");
    assert_eq!(payload.action, "block");
}

#[test]
fn blocking_response_falls_back_to_default_message_for_falsy_detail() {
    let detail = Json::parse("null");
    let response = blocking::build_blocking_response(&detail, "cisco", None, None);
    let [item] = &response.mcp_tool_call_response;
    assert_eq!(
        content_text(&item.text),
        "Blocked by Cisco AI Defense Guardrail"
    );
}

#[test]
fn blocking_response_preserves_response_cost() {
    let detail = Json::parse("null");
    let response = blocking::build_blocking_response(
        &detail,
        "cisco",
        None,
        Some(Decimal::from_str("0.0042").unwrap()),
    );
    assert_eq!(
        response.hidden_params.response_cost,
        Some(Decimal::from_str("0.0042").unwrap())
    );
}

fn replacement() -> Vec<TextContent> {
    vec![TextContent {
        kind: "text",
        text: "replacement text".to_owned(),
    }]
}

#[test]
fn replace_tool_response_updates_wrapped_response() {
    let response = Json::parse(r#"{"mcp_tool_call_response": [{"type": "text", "text": "old"}]}"#);
    let replaced = blocking::replace_tool_response(&response, &replacement()).expect("replaced");
    let ReplacedResponse::ToolCallResponse {
        mcp_tool_call_response,
        ..
    } = &replaced
    else {
        panic!("expected a tool-call-response wrapper");
    };
    assert!(matches!(
        **mcp_tool_call_response,
        ReplacedResponse::Replacement(_)
    ));
    assert_eq!(
        serde_json::to_string(&replaced).unwrap(),
        r#"{"mcp_tool_call_response":[{"type":"text","text":"replacement text"}]}"#
    );
}

#[test]
fn replace_tool_response_updates_content_list_and_structured_mirror() {
    let response = Json::parse(
        r#"{"content": [{"type": "text", "text": "old"}], "structuredContent": {"result": "old"}, "isError": false}"#,
    );
    let replaced = blocking::replace_tool_response(&response, &replacement()).expect("replaced");
    assert_eq!(
        serde_json::to_string(&replaced).unwrap(),
        r#"{"content":[{"type":"text","text":"replacement text"}],"structuredContent":{"result":"replacement text"},"isError":true}"#
    );
}

#[test]
fn replace_tool_response_fills_result_dict_when_missing() {
    let response = Json::parse(r#"{"other": 1}"#);
    let replaced = blocking::replace_tool_response(&response, &replacement()).expect("replaced");
    assert_eq!(
        serde_json::to_string(&replaced).unwrap(),
        r#"{"other":1,"result":{"content":[{"type":"text","text":"replacement text"}],"structuredContent":{"result":"replacement text"},"isError":true}}"#
    );
}

#[test]
fn replace_tool_response_preserves_other_result_keys() {
    let response = Json::parse(r#"{"result": {"content": [], "extra": true}}"#);
    let replaced = blocking::replace_tool_response(&response, &replacement()).expect("replaced");
    assert_eq!(
        serde_json::to_string(&replaced).unwrap(),
        r#"{"result":{"extra":true,"content":[{"type":"text","text":"replacement text"}],"structuredContent":{"result":"replacement text"},"isError":true}}"#
    );
}

#[test]
fn replace_tool_response_replaces_bare_list_wholesale() {
    let response = Json::parse(r#"[{"type": "text", "text": "old"}]"#);
    let replaced = blocking::replace_tool_response(&response, &replacement()).expect("replaced");
    assert!(matches!(replaced, ReplacedResponse::Replacement(_)));
}

#[test]
fn replace_tool_response_rejects_scalar_response() {
    let response = Json::parse("null");
    assert!(blocking::replace_tool_response(&response, &replacement()).is_none());
}

#[test]
fn replacement_structured_content_uses_first_item_text() {
    let mirror = blocking::replacement_structured_content(&replacement()).expect("mirror");
    assert_eq!(mirror.result, "replacement text");
}

#[test]
fn replacement_structured_content_none_for_empty_replacement() {
    assert!(blocking::replacement_structured_content(&[]).is_none());
}

#[test]
fn extract_tool_call_response_unwraps_wrapper() {
    let response = Json::parse(r#"{"mcp_tool_call_response": [{"type": "text", "text": "hi"}]}"#);
    let Json::List(items) = blocking::extract_tool_call_response(&response) else {
        panic!("expected a list");
    };
    assert_eq!(items.len(), 1);
}

#[test]
fn extract_tool_call_response_returns_response_when_unwrapped() {
    let response = Json::parse(r#"{"content": []}"#);
    let Json::Object(original_fields) = &response else {
        panic!("expected an object")
    };
    let Json::Object(inner_fields) = blocking::extract_tool_call_response(&response) else {
        panic!("expected an object")
    };
    assert_eq!(
        serde_json::to_string(&inner_fields).unwrap(),
        serde_json::to_string(original_fields).unwrap()
    );
}

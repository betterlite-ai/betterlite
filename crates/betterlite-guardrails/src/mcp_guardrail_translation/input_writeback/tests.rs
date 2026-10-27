#![cfg(test)]

use crate::mcp_guardrail_translation::process_input::process_input_messages;
use crate::mcp_guardrail_translation::tests::{Reply, ScriptedGuardrail, request, to_json};

#[tokio::test]
async fn masked_argument_leaves_land_in_place_and_keep_the_structure() {
    let mut call = request(
        r#"{"mcp_tool_name":"send","mcp_arguments":{"to":"secret@example.com","count":3,"cc":["ok","secret"]}}"#,
    );
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_input_messages(&mut call, &guardrail).await.unwrap();
    let masked = r#"{"cc":["ok","[MASKED]"],"count":3,"to":"[MASKED]@example.com"}"#;
    assert_eq!(to_json(&call.mcp_arguments), masked);
    assert_eq!(to_json(&call.modified_arguments), masked);
}

#[tokio::test]
async fn a_guardrail_that_changes_nothing_leaves_the_request_as_it_was() {
    let original = request(
        r#"{"mcp_tool_name":"send","mcp_tool_description":"Send mail",
            "mcp_input_schema":{"description":"Mail"},"mcp_arguments":{"to":"a@example.com"},"model":"gpt"}"#,
    );
    let mut call = original.clone();
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_input_messages(&mut call, &guardrail).await.unwrap();
    assert_eq!(call, original);
}

#[tokio::test]
async fn a_guardrail_that_returns_no_texts_leaves_the_request_as_it_was() {
    let original = request(r#"{"mcp_tool_name":"send","mcp_arguments":{"to":"secret"}}"#);
    let mut call = original.clone();
    let guardrail = ScriptedGuardrail::new(Reply::Nothing);
    process_input_messages(&mut call, &guardrail).await.unwrap();
    assert_eq!(call, original);
}

#[tokio::test]
async fn description_and_schema_are_written_back_on_their_own() {
    let mut call = request(
        r#"{"mcp_tool_name":"send","description":"Send secret mail",
            "mcp_input_schema":{"properties":{"to":{"description":"secret address"}}},
            "mcp_arguments":{"to":"a@example.com"}}"#,
    );
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_input_messages(&mut call, &guardrail).await.unwrap();
    assert_eq!(
        call.mcp_tool_description.as_deref(),
        Some("Send [MASKED] mail")
    );
    assert_eq!(call.description.as_deref(), Some("Send secret mail"));
    assert_eq!(
        to_json(&call.mcp_input_schema),
        r#"{"properties":{"to":{"description":"[MASKED] address"}}}"#
    );
    assert_eq!(to_json(&call.mcp_arguments), r#"{"to":"a@example.com"}"#);
    assert_eq!(call.modified_arguments, None);
}

#[tokio::test]
async fn arguments_read_from_the_fallback_field_are_written_to_mcp_arguments() {
    let mut call = request(r#"{"name":"send","arguments":{"to":"secret"}}"#);
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_input_messages(&mut call, &guardrail).await.unwrap();
    assert_eq!(to_json(&call.mcp_arguments), r#"{"to":"[MASKED]"}"#);
    assert_eq!(to_json(&call.modified_arguments), r#"{"to":"[MASKED]"}"#);
    assert_eq!(to_json(&call.arguments), r#"{"to":"secret"}"#);
}

#[tokio::test]
async fn the_proxy_keys_of_the_request_pass_through() {
    let mut call = request(
        r#"{"mcp_tool_name":"send","mcp_arguments":{"to":"secret"},"metadata":{"user":"u1"}}"#,
    );
    let guardrail = ScriptedGuardrail::new(Reply::Mask("secret"));
    process_input_messages(&mut call, &guardrail).await.unwrap();
    assert_eq!(
        to_json(&call),
        r#"{"mcp_tool_name":"send","mcp_arguments":{"to":"[MASKED]"},"modified_arguments":{"to":"[MASKED]"},"metadata":{"user":"u1"}}"#
    );
}

#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "json! fixtures state expected provider JSON directly"
    )
)]

use rstest::rstest;
use serde_json::{Map, Value, json};

use super::error::HistoryError;
use super::history::{HistoryMessage, convert_history};
use super::metadata::{
    IncompleteFinish, finish_reason_for_incomplete, incomplete_reason, provider_metadata,
    shared_output_metadata,
};
use super::reasoning::{
    ReasoningIds, ReasoningOutputItem, chat_reasoning_items, reasoning_input_items,
    reasoning_records,
};
use super::tool_fields::output_tool_fields;

const ENCODED_RESPONSE_ID: &str = "resp_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDpncHQtNG87cmVzcG9uc2VfaWQ6cmVzcF9h";

fn messages(value: Value) -> Vec<HistoryMessage> {
    serde_json::from_value(value).unwrap()
}

fn message(value: Value) -> HistoryMessage {
    serde_json::from_value(value).unwrap()
}

fn object(value: &Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn output_items(value: Value) -> Vec<ReasoningOutputItem> {
    serde_json::from_value(value).unwrap()
}

fn message_item(role: &str, content: &Value) -> Value {
    json!({"type": "message", "role": role, "content": content})
}

#[test]
fn leading_system_messages_join_into_instructions() {
    let converted = convert_history(&messages(json!([
        {"role": "system", "content": "a"},
        {"role": "system", "content": "b"},
        {"role": "user", "content": "c"},
        {"role": "system", "content": "d"}
    ])))
    .unwrap();
    assert_eq!(converted.instructions.as_deref(), Some("a b"));
    assert_eq!(
        converted.input,
        vec![
            message_item("user", &json!([{"type": "input_text", "text": "c"}])),
            message_item("system", &json!([{"type": "input_text", "text": "d"}])),
        ]
    );
}

#[test]
fn empty_leading_system_text_is_replaced_not_joined() {
    let converted = convert_history(&messages(json!([
        {"role": "system", "content": ""},
        {"role": "system", "content": "b"}
    ])))
    .unwrap();
    assert_eq!(converted.instructions.as_deref(), Some("b"));
    assert!(converted.input.is_empty());
}

#[test]
fn a_later_empty_leading_system_keeps_the_join_space() {
    let converted = convert_history(&messages(json!([
        {"role": "system", "content": "a"},
        {"role": "system", "content": ""},
        {"role": "user", "content": "u"}
    ])))
    .unwrap();
    assert_eq!(converted.instructions.as_deref(), Some("a "));
}

#[test]
fn nonstring_leading_system_content_stays_an_input_message() {
    let converted = convert_history(&messages(json!([
        {"role": "system", "content": [{"type": "text", "text": "a"}]},
        {"role": "user", "content": "c"}
    ])))
    .unwrap();
    assert_eq!(converted.instructions, None);
    assert_eq!(
        converted.input,
        vec![
            message_item("system", &json!([{"type": "input_text", "text": "a"}])),
            message_item("user", &json!([{"type": "input_text", "text": "c"}])),
        ]
    );
}

#[test]
fn system_only_history_keeps_instructions_and_empty_input() {
    let converted =
        convert_history(&messages(json!([{"role": "system", "content": "rules"}]))).unwrap();
    assert_eq!(converted.instructions.as_deref(), Some("rules"));
    assert!(converted.input.is_empty());
}

#[test]
fn assistant_calls_keep_reasoning_content_and_call_order() {
    let converted = convert_history(&messages(json!([
        {
            "role": "assistant",
            "content": "hi",
            "reasoning_items": [
                {"id": "rs_a", "type": "reasoning", "summary": [{"type": "summary_text", "text": "s"}]}
            ],
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}},
                {"id": "c2", "type": "custom", "custom": {"name": "g", "input": "x"}}
            ]
        }
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![
            json!({"type": "reasoning", "id": "rs_a", "summary": [{"type": "summary_text", "text": "s"}]}),
            message_item("assistant", &json!([{"type": "output_text", "text": "hi"}])),
            json!({"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"}),
            json!({"type": "custom_tool_call", "call_id": "c2", "name": "g", "input": "x"}),
        ]
    );
}

#[test]
fn assistant_call_without_payload_reports_unsupported() {
    let error = convert_history(&messages(json!([
        {"role": "assistant", "content": "x", "tool_calls": [{"id": "c"}]}
    ])))
    .unwrap_err();
    assert!(matches!(error, HistoryError::UnsupportedToolCall { .. }));
    assert_eq!(error.to_string(), "tool call not supported: {'id': 'c'}");
}

#[test]
fn empty_function_payload_falls_through_to_custom() {
    let converted = convert_history(&messages(json!([
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [{"id": "c9", "function": {}, "custom": {"name": "g", "input": "i"}}]
        }
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![json!({"type": "custom_tool_call", "call_id": "c9", "name": "g", "input": "i"})]
    );
}

#[test]
fn truthy_function_call_wins_over_custom() {
    let converted = convert_history(&messages(json!([
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [{"id": "c9", "function": {"name": "f"}, "custom": {"name": "g"}}]
        },
        {"role": "tool", "tool_call_id": "c9", "content": "ok"}
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![
            json!({"type": "function_call", "call_id": "c9", "name": "f"}),
            json!({"type": "function_call_output", "call_id": "c9", "output": [{"type": "input_text", "text": "ok"}]}),
        ]
    );
}

#[test]
fn assistant_null_content_with_reasoning_emits_only_reasoning() {
    let converted = convert_history(&messages(json!([
        {
            "role": "assistant",
            "content": null,
            "reasoning_items": [{"id": "rs_a", "type": "reasoning", "summary": null}]
        }
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![json!({"type": "reasoning", "id": "rs_a", "summary": []})]
    );
}

#[test]
fn assistant_empty_text_without_calls_emits_empty_output_text() {
    let converted =
        convert_history(&messages(json!([{"role": "assistant", "content": ""}]))).unwrap();
    assert_eq!(
        converted.input,
        vec![message_item(
            "assistant",
            &json!([{"type": "output_text", "text": ""}])
        )]
    );
}

#[test]
fn non_assistant_null_content_emits_nothing() {
    let converted = convert_history(&messages(json!([{"role": "user", "content": null}]))).unwrap();
    assert!(converted.input.is_empty());
    assert_eq!(converted.instructions, None);
}

#[test]
fn empty_tool_calls_are_not_a_call_history() {
    let converted = convert_history(&messages(json!([
        {"role": "assistant", "content": "", "tool_calls": []}
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![message_item(
            "assistant",
            &json!([{"type": "output_text", "text": ""}])
        )]
    );
}

#[test]
fn tool_calls_on_a_non_assistant_message_stay_a_plain_message() {
    let converted = convert_history(&messages(json!([
        {
            "role": "user",
            "content": "hi",
            "tool_calls": [{"id": "c1", "custom": {"name": "g"}}]
        }
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![message_item(
            "user",
            &json!([{"type": "input_text", "text": "hi"}])
        )]
    );
}

#[test]
fn non_assistant_reasoning_items_are_ignored() {
    let converted = convert_history(&messages(json!([
        {
            "role": "user",
            "content": "hi",
            "reasoning_items": [{"id": "rs_a", "type": "reasoning", "summary": []}]
        }
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![message_item(
            "user",
            &json!([{"type": "input_text", "text": "hi"}])
        )]
    );
}

#[test]
fn tool_outputs_are_classified_by_custom_call_ids() {
    let converted = convert_history(&messages(json!([
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [{"id": "c2", "custom": {"name": "g"}}]
        },
        {"role": "tool", "tool_call_id": "c1", "content": "ok"},
        {"role": "tool", "tool_call_id": "c2", "content": "ok"},
        {"role": "tool", "tool_call_id": "c3", "content": null},
        {"role": "tool", "tool_call_id": "c4", "content": 7},
        {"role": "tool", "content": "missing"},
        {"role": "tool", "tool_call_id": "c5", "content": [{"type": "text", "text": "multi"}]}
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![
            json!({"type": "custom_tool_call", "call_id": "c2", "name": "g", "input": ""}),
            json!({"type": "function_call_output", "call_id": "c1", "output": [{"type": "input_text", "text": "ok"}]}),
            json!({"type": "custom_tool_call_output", "call_id": "c2", "output": "ok"}),
            json!({"type": "function_call_output", "call_id": "c3", "output": []}),
            json!({"type": "function_call_output", "call_id": "c4", "output": [{"type": "input_text", "text": "7"}]}),
            json!({"type": "function_call_output", "call_id": null, "output": [{"type": "input_text", "text": "missing"}]}),
            json!({"type": "function_call_output", "call_id": "c5", "output": [{"type": "input_text", "text": "multi"}]}),
        ]
    );
}

#[test]
fn tool_output_before_its_definition_is_still_custom() {
    let converted = convert_history(&messages(json!([
        {"role": "tool", "tool_call_id": "c2", "content": "ok"},
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [{"id": "c2", "custom": {"name": "g"}}]
        }
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![
            json!({"type": "custom_tool_call_output", "call_id": "c2", "output": "ok"}),
            json!({"type": "custom_tool_call", "call_id": "c2", "name": "g", "input": ""}),
        ]
    );
}

#[test]
fn a_tool_message_with_a_multimodal_list_uses_the_content_projection() {
    let converted = convert_history(&messages(json!([
        {
            "role": "tool",
            "tool_call_id": "c",
            "content": [{"type": "text", "text": "a"}, {"type": "image_url", "image_url": {"url": "https://x.test/i"}}]
        }
    ])))
    .unwrap();
    assert_eq!(
        converted.input,
        vec![json!({
            "type": "function_call_output",
            "call_id": "c",
            "output": [
                {"type": "input_text", "text": "a"},
                {"image_url": "https://x.test/i", "detail": "auto", "type": "input_image"}
            ]
        })]
    );
}

#[test]
fn stored_reasoning_items_win_over_thinking_blocks() {
    let mut ids = ReasoningIds::default();
    let items = reasoning_input_items(
        &message(json!({
            "role": "assistant",
            "content": null,
            "reasoning_items": [{"id": "rs_stored", "type": "reasoning", "summary": [{"type": "summary_text", "text": "s"}]}],
            "thinking_blocks": [{"type": "thinking", "thinking": "x", "signature": "sig"}]
        })),
        &mut ids,
    )
    .unwrap();
    assert_eq!(
        items,
        vec![
            json!({"type": "reasoning", "id": "rs_stored", "summary": [{"type": "summary_text", "text": "s"}]})
        ]
    );
}

#[test]
fn stored_reasoning_item_defaults_summary_and_drops_falsy_encrypted_content() {
    let mut ids = ReasoningIds::default();
    let items = reasoning_input_items(
        &message(json!({
            "role": "assistant",
            "content": null,
            "reasoning_items": [
                {"id": "rs_a", "type": "reasoning", "summary": null, "encrypted_content": ""},
                {"id": "rs_b", "type": "reasoning", "summary": [{"type": "custom", "text": "u", "extra": 1}], "encrypted_content": "cipher"}
            ]
        })),
        &mut ids,
    )
    .unwrap();
    assert_eq!(
        items,
        vec![
            json!({"type": "reasoning", "id": "rs_a", "summary": []}),
            json!({
                "type": "reasoning",
                "id": "rs_b",
                "summary": [{"type": "custom", "text": "u", "extra": 1}],
                "encrypted_content": "cipher"
            }),
        ]
    );
}

#[test]
fn missing_reasoning_ids_are_opaque_and_reused() {
    let mut ids = ReasoningIds::default();
    let items = reasoning_input_items(
        &message(json!({
            "role": "assistant",
            "content": null,
            "reasoning_items": [
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "same"}]},
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "same"}]},
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "other"}]}
            ]
        })),
        &mut ids,
    )
    .unwrap();
    let first = items[0]["id"].as_str().unwrap();
    let second = items[1]["id"].as_str().unwrap();
    let third = items[2]["id"].as_str().unwrap();
    assert!(first.starts_with("rs_"));
    assert_eq!(first, second);
    assert_ne!(first, third);
}

#[test]
fn an_empty_stored_reasoning_id_gets_an_rs_prefix() {
    let mut ids = ReasoningIds::default();
    let items = reasoning_input_items(
        &message(json!({
            "role": "assistant",
            "content": null,
            "reasoning_items": [{"id": "", "type": "reasoning", "summary": []}]
        })),
        &mut ids,
    )
    .unwrap();
    let id = items[0]["id"].as_str().unwrap();
    assert!(id.starts_with("rs_"));
}

#[test]
fn thinking_blocks_replay_as_summary_items_without_ids() {
    let mut ids = ReasoningIds::default();
    let items = reasoning_input_items(
        &message(json!({
            "role": "assistant",
            "content": null,
            "thinking_blocks": [
                {"type": "thinking", "thinking": "x", "signature": ""},
                {"type": "thinking", "thinking": "y", "signature": "litellm_encrypted_reasoning:cipher"},
                {"type": "redacted_thinking", "data": "litellm_encrypted_reasoning:red"},
                {"type": "thinking", "thinking": "z", "signature": "sig"}
            ]
        })),
        &mut ids,
    )
    .unwrap();
    assert_eq!(
        items,
        vec![
            json!({"type": "reasoning", "summary": [{"type": "summary_text", "text": "x"}]}),
            json!({
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "y"}],
                "encrypted_content": "cipher"
            }),
            json!({"type": "reasoning", "summary": [], "encrypted_content": "red"}),
            json!({"type": "reasoning", "summary": [{"type": "summary_text", "text": "z"}]}),
        ]
    );
}

#[test]
fn reasoning_records_cover_dict_and_response_items() {
    let items = output_items(json!([
        {"kind": "response", "value": {"id": "rs_t", "encrypted_content": "enc", "summary": [{"type": "summary_text", "text": "typed"}]}},
        {"kind": "dict", "value": {"type": "reasoning", "id": "rs_d", "summary": [{"text": "dict"}], "encrypted_content": null}},
        {"kind": "dict", "value": {"type": "message", "role": "assistant"}},
        {"kind": "response_other"}
    ]));
    let records = reasoning_records(&items).unwrap();
    assert_eq!(
        serde_json::to_value(&records).unwrap(),
        json!([
            {"type": "reasoning", "id": "rs_t", "encrypted_content": "enc", "summary": [{"type": "summary_text", "text": "typed"}]},
            {"type": "reasoning", "id": "rs_d", "encrypted_content": null, "summary": [{"type": "summary_text", "text": "dict"}]}
        ])
    );
}

#[test]
fn missing_dict_reasoning_id_becomes_empty_text() {
    let items =
        output_items(json!([{"kind": "dict", "value": {"type": "reasoning", "summary": []}}]));
    let records = reasoning_records(&items).unwrap();
    assert_eq!(records[0].id, "");
}

#[test]
fn non_reasoning_output_items_are_skipped() {
    let items = output_items(json!([
        {"kind": "dict", "value": {"type": "message"}},
        {"kind": "response_other"}
    ]));
    assert!(reasoning_records(&items).unwrap().is_empty());
}

#[test]
fn empty_reasoning_records_project_to_null() {
    assert_eq!(chat_reasoning_items(&[]), None);
    let items = output_items(json!([
        {"kind": "dict", "value": {"type": "reasoning", "id": "rs_x", "summary": []}}
    ]));
    let records = reasoning_records(&items).unwrap();
    assert_eq!(chat_reasoning_items(&records), Some(records));
}

#[rstest]
#[case::function(
    json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "f", "arguments": "{}"}),
    3,
    json!({"id": "fc_1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 3})
)]
#[case::custom(
    json!({"type": "custom_tool_call", "id": "ct_1", "call_id": "c2", "name": "", "input": "raw"}),
    0,
    json!({"id": "c2", "type": "function", "function": {"name": "custom_tool", "arguments": "raw"}, "index": 0})
)]
#[case::bare_custom(
    json!({"type": "custom_tool_call", "id": "ct_2", "call_id": "c3"}),
    1,
    json!({"id": "c3", "type": "function", "function": {"name": "custom_tool", "arguments": ""}, "index": 1})
)]
#[case::index_shaped_call_id(
    json!({"type": "function_call", "id": "fc_2", "call_id": "call_7"}),
    2,
    json!({"id": "fc_2", "type": "function", "function": {"name": "", "arguments": ""}, "index": 2})
)]
#[case::falsy_function_payload(
    json!({"type": "function_call", "call_id": "c6", "name": null, "arguments": null}),
    3,
    json!({"id": "c6", "type": "function", "function": {"name": "", "arguments": ""}, "index": 3})
)]
fn output_tool_fields_map_function_and_custom_payloads(
    #[case] item: Value,
    #[case] index: usize,
    #[case] expected: Value,
) {
    let converted = output_tool_fields(&object(&item), index).unwrap();
    assert_eq!(serde_json::to_value(converted).unwrap(), expected);
}

#[rstest]
#[case::dict(
    json!({"region": "us"}),
    json!({
        "id": "c4",
        "type": "function",
        "function": {"name": "", "arguments": "", "provider_specific_fields": {"region": "us"}},
        "index": 3,
        "provider_specific_fields": {"region": "us"}
    })
)]
#[case::empty_dict(
    json!({}),
    json!({"id": "c4", "type": "function", "function": {"name": "", "arguments": ""}, "index": 3})
)]
#[case::list(
    json!(["x"]),
    json!({"id": "c4", "type": "function", "function": {"name": "", "arguments": ""}, "index": 3})
)]
fn output_tool_fields_attach_only_nonempty_object_provider_fields(
    #[case] fields: Value,
    #[case] expected: Value,
) {
    let mut item = object(&json!({"type": "function_call", "call_id": "c4"}));
    item.insert("provider_specific_fields".to_owned(), fields);
    let converted = output_tool_fields(&item, 3).unwrap();
    assert_eq!(serde_json::to_value(converted).unwrap(), expected);
}

#[rstest]
#[case::arguments(json!({"type": "function_call", "call_id": "c", "arguments": 7}), "arguments")]
#[case::name(json!({"type": "function_call", "call_id": "c", "name": 7}), "name")]
fn output_tool_fields_reject_nonstring_payloads(#[case] item: Value, #[case] field: &'static str) {
    let error = output_tool_fields(&object(&item), 0).unwrap_err();
    assert!(matches!(error, HistoryError::ToolField { field: rejected } if rejected == field));
}

#[test]
fn provider_metadata_removes_null_chat_and_responses_fields() {
    let fields = json!({
        "id": "bad",
        "output": [],
        "instructions": "x",
        "choices": [],
        "usage": {"total_tokens": 1},
        "status": "completed",
        "vendor": 7,
        "nested": {"a": 1},
        "empty": null
    });
    assert_eq!(
        provider_metadata(fields.as_object()),
        object(&json!({"vendor": 7, "nested": {"a": 1}}))
    );
    assert!(provider_metadata(None).is_empty());
}

#[test]
fn shared_output_metadata_keeps_a_null_id_and_unwraps_an_encoded_one() {
    let null_id = shared_output_metadata(None, None, &json!({}));
    assert_eq!(null_id.upstream_response_id, None);
    assert!(null_id.metadata.is_empty());
    let encoded = shared_output_metadata(None, Some(ENCODED_RESPONSE_ID), &json!({}));
    assert_eq!(encoded.upstream_response_id, Some("resp_a".to_owned()));
    let plain = shared_output_metadata(None, Some("resp_a"), &json!({}));
    assert_eq!(plain.upstream_response_id, Some("resp_a".to_owned()));
}

#[test]
fn shared_output_metadata_carries_the_incomplete_finish_reason() {
    let filtered = shared_output_metadata(
        None,
        None,
        &json!({"incomplete_details": {"reason": "content_filter"}}),
    );
    assert_eq!(
        filtered.incomplete_reason.as_deref(),
        Some("content_filter")
    );
    assert_eq!(filtered.incomplete_finish, IncompleteFinish::ContentFilter);
    let truncated = shared_output_metadata(
        None,
        None,
        &json!({"incomplete_details": {"reason": "max_output_tokens"}}),
    );
    assert_eq!(truncated.incomplete_finish, IncompleteFinish::Length);
}

#[rstest]
#[case::content_filter(
    json!({"incomplete_details": {"reason": "content_filter"}}),
    Some("content_filter"),
    IncompleteFinish::ContentFilter
)]
#[case::max_output_tokens(
    json!({"incomplete_details": {"reason": "max_output_tokens"}}),
    Some("max_output_tokens"),
    IncompleteFinish::Length
)]
#[case::unknown_reason(
    json!({"incomplete_details": {"reason": "unknown"}}),
    Some("unknown"),
    IncompleteFinish::Length
)]
#[case::nonstring_reason(
    json!({"incomplete_details": {"reason": 7}}),
    None,
    IncompleteFinish::Length
)]
#[case::null_details(
    json!({"incomplete_details": null}),
    None,
    IncompleteFinish::Length
)]
#[case::nonmapping_details(
    json!({"incomplete_details": "x"}),
    None,
    IncompleteFinish::Length
)]
#[case::absent_details(json!({}), None, IncompleteFinish::Length)]
#[case::nonmapping_payload(json!("not-a-map"), None, IncompleteFinish::Length)]
fn incomplete_reason_maps_the_documented_finish_reasons(
    #[case] payload: Value,
    #[case] reason: Option<&str>,
    #[case] finish: IncompleteFinish,
) {
    assert_eq!(incomplete_reason(&payload), reason.map(str::to_owned));
    assert_eq!(finish_reason_for_incomplete(reason), finish);
}

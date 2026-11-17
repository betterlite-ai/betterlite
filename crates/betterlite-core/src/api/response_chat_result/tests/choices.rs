#![cfg(test)]

use serde_json::{Value, json};

use crate::api::response_chat_history::metadata::IncompleteFinish;
use crate::api::response_chat_result::choices::output_choices;
use crate::api::response_chat_result::error::ResponseChatResultError;
use crate::api::response_chat_result::reasoning::empty_incomplete_choice;

use super::support::{
    chat_message, decoded, envelope, output_text, reasoning_item, reasoning_record, summary_block,
    text_choice,
};

fn text_with_annotations(content: &str, annotations: Value) -> Value {
    json!({"type": "output_text", "text": content, "annotations": annotations})
}

fn chunk(id: &str, name: &str, arguments: &str, index: usize) -> Value {
    json!({
        "id": id,
        "type": "function",
        "function": {"name": name, "arguments": arguments},
        "index": index,
    })
}

fn tool_choice(calls: Value, index: usize) -> Value {
    json!({
        "finish_reason": "tool_calls",
        "index": index,
        "message": {
            "content": null,
            "function_call": null,
            "provider_specific_fields": null,
            "role": "assistant",
            "tool_calls": calls,
        },
    })
}

fn choices_of(output: Value) -> Value {
    let response = decoded(envelope(output));
    let choices = output_choices(&response.output).expect("the output converts");
    serde_json::to_value(&choices).expect("the choices serialize")
}

fn refusal_of(output: Value) -> ResponseChatResultError {
    let response = decoded(envelope(output));
    output_choices(&response.output).expect_err("the output is refused")
}

fn fallback(output: Value, finish: IncompleteFinish) -> Value {
    let response = decoded(envelope(output));
    let choice = empty_incomplete_choice(&response.output, finish).expect("the fallback builds");
    serde_json::to_value(&choice).expect("the choice serializes")
}

#[test]
fn typed_message_emits_one_choice_per_block() {
    let choices = choices_of(json!([{
        "type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
        "content": [output_text("a"), output_text("b")],
    }]));
    assert_eq!(
        choices,
        json!([
            text_choice("a", "assistant", 0),
            text_choice("b", "assistant", 1),
        ])
    );
}

#[test]
fn typed_refusal_has_empty_content() {
    let choices = choices_of(json!([{
        "type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
        "content": [output_text("a"), {"type": "refusal", "refusal": "no"}],
    }]));
    assert_eq!(
        choices,
        json!([
            text_choice("a", "assistant", 0),
            text_choice("", "assistant", 1),
        ])
    );
}

#[test]
fn typed_annotations_are_kept_unfiltered() {
    let choices = choices_of(json!([{
        "type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
        "content": [text_with_annotations("a", json!([
            {"type": "url_citation", "url": "https://x", "title": "t", "start_index": 1, "end_index": 2},
            {"type": "file_citation", "file_id": "f1", "filename": "a.txt", "index": 1},
        ]))],
    }]));
    let mut expected = chat_message("a", "assistant");
    expected["annotations"] = json!([
        {"type": "url_citation", "url": "https://x", "title": "t", "start_index": 1, "end_index": 2},
        {"type": "file_citation", "file_id": "f1", "filename": "a.txt", "index": 1},
    ]);
    assert_eq!(
        choices,
        json!([{"finish_reason": "stop", "index": 0, "message": expected}])
    );
}

#[test]
fn typed_empty_annotations_are_absent() {
    let choices = choices_of(json!([{
        "type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
        "content": [output_text("a")],
    }]));
    assert_eq!(choices, json!([text_choice("a", "assistant", 0)]));
}

#[test]
fn raw_message_keeps_only_the_first_output_text_block() {
    let choices = choices_of(json!([{
        "type": "message", "role": "user",
        "content": [output_text("a"), output_text("b")],
    }]));
    assert_eq!(choices, json!([text_choice("a", "user", 0)]));
}

#[test]
fn raw_message_missing_role_defaults_assistant() {
    let choices = choices_of(json!([{
        "type": "message",
        "content": [{"type": "output_text", "text": "a"}],
    }]));
    assert_eq!(choices, json!([text_choice("a", "assistant", 0)]));
}

#[test]
fn raw_message_missing_text_is_empty() {
    let choices = choices_of(json!([{
        "type": "message", "role": "assistant",
        "content": [{"type": "output_text"}],
    }]));
    assert_eq!(choices, json!([text_choice("", "assistant", 0)]));
}

#[test]
fn raw_message_falsy_text_is_empty() {
    for falsy in [json!(""), json!(0), json!([]), json!(false), json!(null)] {
        let choices = choices_of(json!([{
            "type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": falsy}],
        }]));
        assert_eq!(choices, json!([text_choice("", "assistant", 0)]));
    }
}

#[test]
fn raw_message_truthy_non_string_text_is_refused() {
    let error = refusal_of(json!([{
        "type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": 5}],
    }]));
    assert!(matches!(
        error,
        ResponseChatResultError::RawField { field: "text" }
    ));
}

#[test]
fn raw_message_refusal_is_skipped() {
    let choices = choices_of(json!([{
        "type": "message", "role": "assistant",
        "content": [{"type": "refusal", "refusal": "no"}],
    }]));
    assert_eq!(choices, json!([]));
}

#[test]
fn raw_message_takes_the_first_valid_text_part() {
    let choices = choices_of(json!([
        {"type": "message", "role": "assistant", "content": [{"type": "input_text", "text": "a"}]},
        {"type": "message", "role": "assistant", "content": [5, {"type": "output_text", "text": "b"}]},
    ]));
    assert_eq!(choices, json!([text_choice("b", "assistant", 0)]));
}

#[test]
fn raw_message_null_content_is_skipped() {
    let choices = choices_of(json!([{"type": "message", "role": "assistant", "content": null}]));
    assert_eq!(choices, json!([]));
}

#[test]
fn raw_message_content_not_a_list_is_refused() {
    let error = refusal_of(json!([{"type": "message", "role": "assistant", "content": "a"}]));
    assert!(matches!(
        error,
        ResponseChatResultError::RawField { field: "content" }
    ));
}

#[test]
fn raw_message_unknown_role_is_refused() {
    let error = refusal_of(json!([{
        "type": "message", "role": "narrator",
        "content": [{"type": "output_text", "text": "a"}],
    }]));
    assert!(matches!(
        error,
        ResponseChatResultError::RawField { field: "role" }
    ));
}

#[test]
fn raw_message_roles_map_to_chat_roles() {
    for (role, expected) in [
        ("", "assistant"),
        ("assistant", "assistant"),
        ("user", "user"),
        ("system", "system"),
        ("tool", "tool"),
        ("function", "function"),
    ] {
        let choices = choices_of(json!([{
            "type": "message", "role": role,
            "content": [{"type": "output_text", "text": "a"}],
        }]));
        assert_eq!(choices, json!([text_choice("a", expected, 0)]));
    }
}

#[test]
fn raw_message_annotations_are_kept_unfiltered() {
    let choices = choices_of(json!([{
        "type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": "a", "annotations": [
            {"type": "file_citation", "file_id": "f1"},
            {"x": 1},
            5,
        ]}],
    }]));
    let mut expected = chat_message("a", "assistant");
    expected["annotations"] = json!([{"type": "file_citation", "file_id": "f1"}, {"x": 1}]);
    assert_eq!(
        choices,
        json!([{"finish_reason": "stop", "index": 0, "message": expected}])
    );
}

#[test]
fn raw_message_annotations_not_a_list_are_refused() {
    let error = refusal_of(json!([{
        "type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": "a", "annotations": 5}],
    }]));
    assert!(matches!(
        error,
        ResponseChatResultError::RawField {
            field: "annotations"
        }
    ));
}

#[test]
fn unknown_items_create_no_choice() {
    let choices = choices_of(json!([
        {"type": "reasoning", "id": "rs_1", "summary": [{"text": "raw"}]},
        {"type": "file_search_call", "id": "fs_1", "status": "completed", "queries": []},
    ]));
    assert_eq!(choices, json!([]));
}

#[test]
fn typed_reasoning_flushes_onto_the_first_text_choice() {
    let choices = choices_of(json!([
        reasoning_item("rs_1", json!([summary_block("one"), summary_block("two")])),
        {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
         "content": [output_text("a"), output_text("b")]},
    ]));
    let mut first = chat_message("a", "assistant");
    first["reasoning_content"] = json!("one two");
    first["reasoning_items"] = json!([reasoning_record(
        "rs_1",
        json!([summary_block("one"), summary_block("two")])
    )]);
    assert_eq!(
        choices,
        json!([
            {"finish_reason": "stop", "index": 0, "message": first},
            text_choice("b", "assistant", 1),
        ])
    );
}

#[test]
fn typed_reasoning_replaces_rather_than_concatenates() {
    let choices = choices_of(json!([
        reasoning_item("rs_1", json!([summary_block("one")])),
        reasoning_item("rs_2", json!([summary_block("two")])),
        {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
         "content": [output_text("a")]},
    ]));
    let mut first = chat_message("a", "assistant");
    first["reasoning_content"] = json!("two");
    first["reasoning_items"] = json!([reasoning_record("rs_2", json!([summary_block("two")]))]);
    assert_eq!(
        choices,
        json!([{"finish_reason": "stop", "index": 0, "message": first}])
    );
}

#[test]
fn typed_reasoning_without_summary_keeps_an_empty_string() {
    let choices = choices_of(json!([
        reasoning_item("rs_1", json!([])),
        {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
         "content": [output_text("a")]},
    ]));
    let mut first = chat_message("a", "assistant");
    first["reasoning_content"] = json!("");
    first["reasoning_items"] = json!([reasoning_record("rs_1", json!([]))]);
    assert_eq!(
        choices,
        json!([{"finish_reason": "stop", "index": 0, "message": first}])
    );
}

#[test]
fn raw_reasoning_does_not_reach_the_text_choice() {
    let choices = choices_of(json!([
        {"type": "reasoning", "id": "rs_1", "summary": [{"text": "raw"}]},
        {"type": "message", "role": "assistant", "content": [output_text("a")]},
    ]));
    assert_eq!(choices, json!([text_choice("a", "assistant", 0)]));
}

#[test]
fn typed_function_call_goes_into_a_trailing_choice() {
    let choices = choices_of(json!([
        {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
         "content": [output_text("a")]},
        {"type": "function_call", "id": "fc_1", "call_id": "call_abc", "name": "f", "arguments": "{}"},
    ]));
    assert_eq!(
        choices,
        json!([
            text_choice("a", "assistant", 0),
            tool_choice(json!([chunk("call_abc", "f", "{}", 0)]), 1),
        ])
    );
}

#[test]
fn typed_function_calls_keep_the_index_zero_behavior() {
    let choices = choices_of(json!([
        {"type": "function_call", "id": "fc_1", "call_id": "c1", "name": "f", "arguments": "{}"},
        {"type": "function_call", "id": "fc_2", "call_id": "c2", "name": "g", "arguments": "{}"},
    ]));
    assert_eq!(
        choices,
        json!([tool_choice(
            json!([chunk("c1", "f", "{}", 0), chunk("c2", "g", "{}", 0)]),
            0
        )])
    );
}

#[test]
fn raw_function_calls_use_sequential_indices() {
    let choices = choices_of(json!([
        {"type": "function_call", "id": "fc_1", "name": "f", "arguments": "{}"},
        {"type": "function_call", "id": "fc_2", "name": "g", "arguments": "{}"},
    ]));
    assert_eq!(
        choices,
        json!([tool_choice(
            json!([chunk("fc_1", "f", "{}", 0), chunk("fc_2", "g", "{}", 1)]),
            0
        )])
    );
}

#[test]
fn raw_custom_tool_call_carries_its_input() {
    let choices = choices_of(json!([{
        "type": "custom_tool_call", "id": "ctc_1", "name": "g", "input": "raw",
    }]));
    assert_eq!(
        choices,
        json!([tool_choice(json!([chunk("ctc_1", "g", "raw", 0)]), 0)])
    );
}

#[test]
fn typed_apply_patch_dumps_its_operation() {
    let choices = choices_of(json!([{
        "type": "apply_patch_call", "id": "apc_1", "call_id": "call_ap", "status": "completed",
        "operation": {"type": "create_file", "diff": "+a", "path": "a", "zz": 1},
    }]));
    let arguments = r#"{"diff": "+a", "path": "a", "type": "create_file", "zz": 1}"#;
    assert_eq!(
        choices,
        json!([tool_choice(
            json!([chunk("call_ap", "apply_patch", arguments, 0)]),
            0
        )])
    );
}

#[test]
fn tool_calls_carry_unflushed_reasoning() {
    let choices = choices_of(json!([
        reasoning_item("rs_1", json!([summary_block("why")])),
        {"type": "function_call", "id": "fc_1", "call_id": "c1", "name": "f", "arguments": "{}"},
    ]));
    let mut trailing = tool_choice(json!([chunk("c1", "f", "{}", 0)]), 0);
    trailing["message"]["reasoning_content"] = json!("why");
    trailing["message"]["reasoning_items"] =
        json!([reasoning_record("rs_1", json!([summary_block("why")]))]);
    assert_eq!(choices, json!([trailing]));
}

#[test]
fn text_flushes_reasoning_so_the_tool_choice_does_not_carry_it() {
    let choices = choices_of(json!([
        reasoning_item("rs_1", json!([summary_block("why")])),
        {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
         "content": [output_text("a")]},
        {"type": "function_call", "id": "fc_1", "call_id": "c1", "name": "f", "arguments": "{}"},
    ]));
    let mut first = chat_message("a", "assistant");
    first["reasoning_content"] = json!("why");
    first["reasoning_items"] = json!([reasoning_record("rs_1", json!([summary_block("why")]))]);
    assert_eq!(
        choices,
        json!([
            {"finish_reason": "stop", "index": 0, "message": first},
            tool_choice(json!([chunk("c1", "f", "{}", 0)]), 1),
        ])
    );
}

#[test]
fn fallback_collects_typed_and_raw_reasoning() {
    let choice = fallback(
        json!([
            reasoning_item("rs_1", json!([summary_block("one")])),
            {"type": "reasoning", "summary": [{"text": "three"}]},
        ]),
        IncompleteFinish::Length,
    );
    let mut expected = chat_message("", "assistant");
    expected["reasoning_content"] = json!("one three");
    expected["reasoning_items"] = json!([
        reasoning_record("rs_1", json!([summary_block("one")])),
        reasoning_record("", json!([summary_block("three")])),
    ]);
    assert_eq!(
        choice,
        json!({"finish_reason": "length", "index": 0, "message": expected})
    );
}

#[test]
fn fallback_joins_only_nonempty_summaries() {
    let blocks = json!([summary_block(""), summary_block("x"), summary_block("")]);
    let choice = fallback(
        json!([reasoning_item("rs_1", blocks.clone())]),
        IncompleteFinish::ContentFilter,
    );
    let mut expected = chat_message("", "assistant");
    expected["reasoning_content"] = json!("x");
    expected["reasoning_items"] = json!([reasoning_record("rs_1", blocks)]);
    assert_eq!(
        choice,
        json!({"finish_reason": "content_filter", "index": 0, "message": expected})
    );
}

#[test]
fn fallback_without_reasoning_has_no_reasoning_fields() {
    let choice = fallback(
        json!([{"type": "message", "role": "assistant", "content": [output_text("a")]}]),
        IncompleteFinish::Length,
    );
    assert_eq!(
        choice,
        json!({"finish_reason": "length", "index": 0, "message": chat_message("", "assistant")})
    );
}

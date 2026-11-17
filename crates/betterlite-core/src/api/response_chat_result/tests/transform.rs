#![cfg(test)]

use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use rstest::rstest;
use serde_json::{Value, json};

use crate::api::response_chat_result::error::ResponseChatResultError;
use crate::api::response_chat_result::records::ModelResponse;

use super::support::{
    MODEL, caller, caller_with_hidden, chat_message, choices_value, decoded, decoded_with,
    envelope, object, output_text, reasoning_item, reasoning_record, recorded_state, run, run_with,
    state, summary_block, text_choice,
};

const ENCODED_ID: &str = "resp_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDpncHQtNG87cmVzcG9uc2VfaWQ6cmVzcF9h";
const RECOVERED: &str =
    r#"[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}]"#;

fn typed_message(content: &str) -> Value {
    json!({
        "type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
        "content": [output_text(content)],
    })
}

fn tool_call() -> Value {
    json!({"type": "function_call", "id": "fc_1", "call_id": "c1", "name": "f", "arguments": "{}"})
}

fn record_of(response: &mut ResponsesApiResponse) -> ModelResponse {
    let (result, _) = run(response, &state());
    result.expect("the response converts")
}

#[test]
fn transform_sets_identity_and_keeps_caller_fields() {
    let mut response = decoded(envelope(json!([typed_message("a")])));
    let (result, lines) = run(&mut response, &state());
    let record = result.expect("the response converts");
    assert_eq!(record.id, "resp_plain");
    assert_eq!(record.model.as_deref(), Some(MODEL));
    assert_eq!(record.created, 5_i64);
    assert_eq!(record.object, "chat.completion");
    assert_eq!(record.system_fingerprint.as_deref(), Some("fp"));
    assert_eq!(
        choices_value(&record),
        json!([text_choice("a", "assistant", 0)])
    );
    assert!(lines.is_empty());
}

#[test]
fn transform_decodes_an_encoded_response_id() {
    let mut response = decoded_with(json!([typed_message("a")]), json!({"id": ENCODED_ID}));
    assert_eq!(record_of(&mut response).id, "resp_a");
}

#[test]
fn transform_reports_the_response_error_before_recovery() {
    let mut response = decoded_with(json!([]), json!({"error": {"message": "bad"}}));
    let (result, lines) = run(&mut response, &recorded_state(RECOVERED));
    let error = result.expect_err("the response errors");
    assert_eq!(error.to_string(), r#"Error in response: {"message":"bad"}"#);
    assert!(lines.is_empty());
}

#[test]
fn transform_prints_an_empty_error_body_as_an_empty_object() {
    let mut response = decoded_with(json!([]), json!({"error": {}}));
    let (result, _) = run(&mut response, &state());
    let error = result.expect_err("the response errors");
    assert_eq!(error.to_string(), "Error in response: {}");
}

#[test]
fn transform_recovers_a_recorded_terminal_message() {
    let mut response = decoded(envelope(json!([])));
    let (result, lines) = run(&mut response, &recorded_state(RECOVERED));
    let record = result.expect("the recovered response converts");
    assert_eq!(
        choices_value(&record),
        json!([text_choice("hi", "assistant", 0)])
    );
    assert_eq!(response.output.len(), 1);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0]
            .contains("Recovered empty Responses API output from raw SSE for model=caller-model")
    );
}

#[test]
fn transform_drops_recovered_items_that_are_not_objects() {
    let mut response = decoded(envelope(json!([])));
    let recording = r#"[1,{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}]"#;
    let (result, lines) = run(&mut response, &recorded_state(recording));
    let record = result.expect("the recovered response converts");
    assert_eq!(
        choices_value(&record),
        json!([text_choice("hi", "assistant", 0)])
    );
    assert_eq!(response.output.len(), 1);
    assert_eq!(lines.len(), 1);
}

#[test]
fn transform_ignores_the_recording_when_output_is_present() {
    let mut response = decoded(envelope(json!([typed_message("a")])));
    let (result, lines) = run(&mut response, &recorded_state(RECOVERED));
    let record = result.expect("the response converts");
    assert_eq!(
        choices_value(&record),
        json!([text_choice("a", "assistant", 0)])
    );
    assert!(lines.is_empty());
    assert_eq!(response.output.len(), 1);
}

#[test]
fn transform_reports_unknown_items_when_nothing_recovers() {
    let mut response = decoded(envelope(json!([])));
    let (result, _) = run(&mut response, &state());
    let error = result.expect_err("the empty output is unknown");
    assert_eq!(
        error.to_string(),
        "Unknown items in responses API response: []"
    );
}

#[test]
fn transform_reports_unknown_items_for_a_failed_status() {
    let mut response = decoded_with(json!([]), json!({"status": "failed"}));
    let (result, _) = run(&mut response, &state());
    let error = result.expect_err("a failed status alone is not incomplete");
    assert!(
        error
            .to_string()
            .starts_with("Unknown items in responses API response:")
    );
}

#[test]
fn transform_builds_an_empty_choice_for_an_incomplete_response() {
    let mut response = decoded_with(
        json!([reasoning_item("rs_1", json!([summary_block("why")]), None)]),
        json!({"status": "incomplete"}),
    );
    let record = record_of(&mut response);
    let mut expected = chat_message("", "assistant");
    expected["reasoning_content"] = json!("why");
    expected["reasoning_items"] = json!([reasoning_record(
        "rs_1",
        json!([summary_block("why")]),
        None
    )]);
    assert_eq!(
        choices_value(&record),
        json!([{"finish_reason": "length", "index": 0, "message": expected}])
    );
}

#[test]
fn transform_builds_an_empty_choice_from_incomplete_details() {
    let mut response = decoded_with(
        json!([]),
        json!({"incomplete_details": {"reason": "max_output_tokens"}}),
    );
    let record = record_of(&mut response);
    assert_eq!(
        choices_value(&record),
        json!([{
            "finish_reason": "length",
            "index": 0,
            "message": chat_message("", "assistant"),
        }])
    );
}

#[test]
fn transform_incomplete_details_override_a_completed_status() {
    let mut response = decoded_with(
        json!([typed_message("a")]),
        json!({
            "status": "completed",
            "incomplete_details": {"reason": "max_output_tokens"},
        }),
    );
    let record = record_of(&mut response);
    let mut expected = text_choice("a", "assistant", 0);
    expected["finish_reason"] = json!("length");
    assert_eq!(choices_value(&record), json!([expected]));
}

#[test]
fn transform_content_filter_overrides_every_finish_reason() {
    let mut response = decoded_with(
        json!([{
            "type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
            "content": [output_text("a"), output_text("b")],
        }]),
        json!({"status": "incomplete", "incomplete_details": {"reason": "content_filter"}}),
    );
    let record = record_of(&mut response);
    let mut first = text_choice("a", "assistant", 0);
    first["finish_reason"] = json!("content_filter");
    let mut second = text_choice("b", "assistant", 1);
    second["finish_reason"] = json!("content_filter");
    assert_eq!(choices_value(&record), json!([first, second]));
}

#[test]
fn transform_incomplete_overrides_the_tool_choice() {
    let mut response = decoded_with(json!([tool_call()]), json!({"status": "incomplete"}));
    let record = record_of(&mut response);
    let choices = choices_value(&record);
    assert_eq!(choices[0]["finish_reason"], json!("length"));
    assert_eq!(choices[0]["message"]["content"], json!(null));
    assert_eq!(
        choices[0]["message"]["tool_calls"][0]["function"]["name"],
        json!("f")
    );
}

#[test]
fn transform_null_incomplete_reason_is_not_incomplete() {
    let mut response = decoded_with(json!([]), json!({"incomplete_details": {"reason": null}}));
    let (result, _) = run(&mut response, &state());
    let error = result.expect_err("a null reason is not incomplete");
    assert_eq!(
        error.to_string(),
        "Unknown items in responses API response: []"
    );
}

#[rstest]
#[case::absent(
    json!(null),
    json!({
        "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0,
        "prompt_tokens_details": null, "completion_tokens_details": null,
    })
)]
#[case::stale_total(
    json!({"input_tokens": 2, "output_tokens": 3, "total_tokens": 99}),
    json!({
        "prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5,
        "prompt_tokens_details": null, "completion_tokens_details": null,
    })
)]
#[case::extras_and_cost(
    json!({
        "input_tokens": 2, "output_tokens": 3, "total_tokens": 5,
        "server_side_tool_usage_details": {"web_search": 1}, "cost": 0.5,
    }),
    json!({
        "prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5,
        "prompt_tokens_details": null, "completion_tokens_details": null,
        "server_side_tool_usage_details": {"web_search": 1}, "cost": 0.5,
    })
)]
fn transform_projects_usage(#[case] usage: Value, #[case] expected: Value) {
    let mut response = decoded_with(json!([typed_message("a")]), json!({"usage": usage}));
    let record = record_of(&mut response);
    assert_eq!(record.usage, Some(expected));
}

#[test]
fn transform_keeps_only_provider_metadata() {
    let mut response = decoded_with(
        json!([typed_message("a")]),
        json!({
            "service_tier": "default",
            "model_id": "m1",
            "hidden": 1,
            "custom_llm_provider": "openai",
            "created": "dropped",
            "system_fingerprint": "dropped",
            "nothing": null,
        }),
    );
    let mut start = caller();
    start.extra = object(json!({"existing": "kept"}));
    let (converted, _) = run_with(start, &mut response, &object(json!({})), &state());
    let record = converted.expect("the response converts");
    assert_eq!(
        record.extra,
        object(json!({
            "existing": "kept",
            "service_tier": "default",
            "model_id": "m1",
            "hidden": 1,
            "custom_llm_provider": "openai",
        }))
    );
}

#[test]
fn transform_merges_hidden_scalars_over_existing_values() {
    let mut response = decoded(envelope(json!([typed_message("a")])));
    let hidden = object(json!({"model_id": "new"}));
    let result = caller_with_hidden(json!({"model_id": "old", "x-vendor": "keep"}));
    let (converted, _) = run_with(result, &mut response, &hidden, &state());
    let record = converted.expect("the response converts");
    assert_eq!(
        record.hidden_params,
        object(json!({"model_id": "new", "x-vendor": "keep"}))
    );
}

#[test]
fn transform_merges_additional_headers_with_existing_winning() {
    let mut response = decoded(envelope(json!([typed_message("a")])));
    let hidden = object(json!({"additional_headers": {"x-request-id": "new", "x-vendor": "v"}}));
    let result = caller_with_hidden(json!({"additional_headers": {"x-request-id": "old"}}));
    let (converted, _) = run_with(result, &mut response, &hidden, &state());
    let record = converted.expect("the response converts");
    assert_eq!(
        record.hidden_params,
        object(json!({"additional_headers": {"x-request-id": "old", "x-vendor": "v"}}))
    );
}

#[test]
fn transform_adds_additional_headers_when_none_exist() {
    let mut response = decoded(envelope(json!([typed_message("a")])));
    let hidden = object(json!({"additional_headers": {"x-vendor": "v"}}));
    let (converted, _) = run_with(caller(), &mut response, &hidden, &state());
    let record = converted.expect("the response converts");
    assert_eq!(
        record.hidden_params,
        object(json!({"additional_headers": {"x-vendor": "v"}}))
    );
}

#[test]
fn transform_leaves_hidden_params_alone_when_the_response_has_none() {
    let mut response = decoded(envelope(json!([typed_message("a")])));
    let result = caller_with_hidden(json!({"model_id": "old"}));
    let (converted, _) = run_with(result, &mut response, &object(json!({})), &state());
    let record = converted.expect("the response converts");
    assert_eq!(record.hidden_params, object(json!({"model_id": "old"})));
}

#[rstest]
#[case::raw(json!({"additional_headers": "x"}), json!({"additional_headers": {}}))]
#[case::existing(json!({"additional_headers": {"a": 1}}), json!({"additional_headers": "x"}))]
fn transform_refuses_hidden_headers_that_are_not_objects(
    #[case] raw: Value,
    #[case] existing: Value,
) {
    let mut response = decoded(envelope(json!([typed_message("a")])));
    let hidden = object(raw);
    let result = caller_with_hidden(existing);
    let (converted, _) = run_with(result, &mut response, &hidden, &state());
    let error = converted.expect_err("the headers are refused");
    assert!(matches!(error, ResponseChatResultError::HiddenHeaders));
}

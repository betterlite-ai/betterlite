#![cfg(test)]

use rstest::rstest;

use super::{OutputResponse, request_input, response_output};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

fn data(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("request data")
}

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

#[rstest]
#[case::messages_as_given(
    r#"{"messages": [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "yo"}]}"#,
    Some(r#"[{"content":"hi","role":"user"},{"content":"yo","role":"assistant"}]"#)
)]
#[case::system_then_instructions_before_the_turns(
    r#"{"instructions": "i", "system": "s", "messages": [{"role": "user", "content": "hi"}]}"#,
    Some(r#"[{"content":"s","role":"system"},{"content":"i","role":"system"},{"content":"hi","role":"user"}]"#)
)]
#[case::null_system_is_skipped(
    r#"{"system": null, "instructions": "i", "messages": []}"#,
    Some(r#"[{"content":"i","role":"system"}]"#)
)]
#[case::empty_turns("{\"messages\": []}", Some("[]"))]
#[case::input_text_is_one_user_turn(
    r#"{"input": "hello"}"#,
    Some(r#"[{"content":"hello","role":"user"}]"#)
)]
#[case::messages_text_is_one_user_turn(
    r#"{"messages": "hello"}"#,
    Some(r#"[{"content":"hello","role":"user"}]"#)
)]
#[case::input_list_is_used_as_given(
    r#"{"input": [{"type": "message", "content": "x"}]}"#,
    Some(r#"[{"content":"x","type":"message"}]"#)
)]
#[case::an_object_is_one_user_turn(
    r#"{"input": {"a": 1}}"#,
    Some(r#"[{"content":{"a":1},"role":"user"}]"#)
)]
#[case::a_number_is_one_user_turn(r#"{"input": 3}"#, Some(r#"[{"content":3,"role":"user"}]"#))]
#[case::messages_win_over_input(
    r#"{"messages": [{"role": "user", "content": "m"}], "input": "i"}"#,
    Some(r#"[{"content":"m","role":"user"}]"#)
)]
#[case::a_null_messages_key_hides_the_input(r#"{"messages": null, "input": "i"}"#, None)]
#[case::null_input(r#"{"input": null}"#, None)]
#[case::neither_key(r#"{"system": "s"}"#, None)]
fn request_input_turns_system_and_user_messages_into_json(
    #[case] request: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(request_input(&data(request)).as_deref(), expected);
}

#[test]
fn a_model_response_projects_each_choice_message_without_null_fields() {
    let response = value(
        r#"{"choices": [
            {"index": 0, "message": {"role": "assistant", "content": "hi", "tool_calls": null, "function_call": null}},
            {"index": 1, "message": {"role": "assistant", "content": null, "tool_calls": [
                {"id": "c", "type": "function", "extra": null, "function": {"name": "f", "arguments": "{}"}}
            ]}}
        ]}"#,
    );
    assert_eq!(
        response_output(&OutputResponse::ModelResponse(&response)).as_deref(),
        Some(
            r#"[{"content":"hi","role":"assistant"},{"role":"assistant","tool_calls":[{"function":{"arguments":"{}","name":"f"},"id":"c","type":"function"}]}]"#
        )
    );
}

#[derive(Clone, Copy)]
enum Class {
    Model,
    Responses,
    Anthropic,
}

#[rstest]
#[case::model_no_role(
    Class::Model,
    r#"{"choices": [{"message": {"content": "hi"}}]}"#,
    Some(r#"[{"content":"hi","role":"assistant"}]"#)
)]
#[case::model_no_message(
    Class::Model,
    r#"{"choices": [{"index": 0}]}"#,
    Some(r#"[{"role":"assistant"}]"#)
)]
#[case::model_no_choices(Class::Model, r#"{"id": "x"}"#, Some(r#"[{"role":"assistant"}]"#))]
#[case::model_null_choices(
    Class::Model,
    r#"{"choices": null}"#,
    Some(r#"[{"role":"assistant"}]"#)
)]
#[case::model_empty_choices(Class::Model, r#"{"choices": []}"#, Some("[]"))]
#[case::model_choices_of_the_wrong_type(Class::Model, r#"{"choices": "x"}"#, None)]
#[case::model_not_an_object(Class::Model, r#"["x"]"#, None)]
#[case::responses_output_list(
    Class::Responses,
    r#"{"id": "r", "output": [{"type": "message", "status": null, "content": [{"type": "output_text", "text": "hi", "logprobs": null}]}]}"#,
    Some(r#"[{"content":[{"text":"hi","type":"output_text"}],"type":"message"}]"#)
)]
#[case::responses_no_output(Class::Responses, r#"{"id": "r"}"#, Some("null"))]
#[case::responses_null_output(Class::Responses, r#"{"output": null}"#, Some("null"))]
#[case::responses_empty_output(Class::Responses, r#"{"output": []}"#, Some("[]"))]
#[case::responses_not_an_object(Class::Responses, r#""text""#, None)]
#[case::anthropic_full(
    Class::Anthropic,
    r#"{"type": "message", "role": "assistant", "content": [{"type": "text", "text": "hi"}], "model": "x"}"#,
    Some(r#"[{"content":[{"text":"hi","type":"text"}],"role":"assistant"}]"#)
)]
#[case::anthropic_role_defaults_to_assistant(
    Class::Anthropic,
    r#"{"type": "message", "content": "hi"}"#,
    Some(r#"[{"content":"hi","role":"assistant"}]"#)
)]
#[case::anthropic_content_defaults_to_null(
    Class::Anthropic,
    r#"{"type": "message", "role": "user"}"#,
    Some(r#"[{"content":null,"role":"user"}]"#)
)]
#[case::anthropic_explicit_null_content(
    Class::Anthropic,
    r#"{"type": "message", "content": null}"#,
    Some(r#"[{"content":null,"role":"assistant"}]"#)
)]
#[case::anthropic_another_type(Class::Anthropic, r#"{"type": "error", "content": "hi"}"#, None)]
#[case::anthropic_no_type(Class::Anthropic, r#"{"role": "assistant", "content": "hi"}"#, None)]
#[case::anthropic_null_role(Class::Anthropic, r#"{"type": "message", "role": null}"#, None)]
#[case::anthropic_numeric_role(Class::Anthropic, r#"{"type": "message", "role": 3}"#, None)]
#[case::anthropic_a_text(Class::Anthropic, r#""message""#, None)]
#[case::anthropic_a_null(Class::Anthropic, "null", None)]
#[case::anthropic_a_list(Class::Anthropic, r#"[{"type": "message"}]"#, None)]
fn a_response_projects_to_the_json_its_class_defines(
    #[case] class: Class,
    #[case] response: &str,
    #[case] expected: Option<&str>,
) {
    let parsed = value(response);
    let wrapped = match class {
        Class::Model => OutputResponse::ModelResponse(&parsed),
        Class::Responses => OutputResponse::ResponsesApi(&parsed),
        Class::Anthropic => OutputResponse::Other(&parsed),
    };
    assert_eq!(response_output(&wrapped).as_deref(), expected);
}

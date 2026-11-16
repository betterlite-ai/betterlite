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

#[rstest]
#[case::no_role(
    r#"{"choices": [{"message": {"content": "hi"}}]}"#,
    Some(r#"[{"content":"hi","role":"assistant"}]"#)
)]
#[case::no_message(r#"{"choices": [{"index": 0}]}"#, Some(r#"[{"role":"assistant"}]"#))]
#[case::no_choices(r#"{"id": "x"}"#, Some(r#"[{"role":"assistant"}]"#))]
#[case::null_choices(r#"{"choices": null}"#, Some(r#"[{"role":"assistant"}]"#))]
#[case::empty_choices(r#"{"choices": []}"#, Some("[]"))]
#[case::choices_of_the_wrong_type(r#"{"choices": "x"}"#, None)]
#[case::not_an_object(r#"["x"]"#, None)]
fn a_model_response_without_the_usual_shape_follows_the_model_defaults(
    #[case] response: &str,
    #[case] expected: Option<&str>,
) {
    let parsed = value(response);
    assert_eq!(
        response_output(&OutputResponse::ModelResponse(&parsed)).as_deref(),
        expected
    );
}

#[rstest]
#[case::output_list(
    r#"{"id": "r", "output": [{"type": "message", "status": null, "content": [{"type": "output_text", "text": "hi", "logprobs": null}]}]}"#,
    Some(r#"[{"content":[{"text":"hi","type":"output_text"}],"type":"message"}]"#)
)]
#[case::no_output(r#"{"id": "r"}"#, Some("null"))]
#[case::null_output(r#"{"output": null}"#, Some("null"))]
#[case::empty_output(r#"{"output": []}"#, Some("[]"))]
#[case::not_an_object(r#""text""#, None)]
fn a_responses_api_response_projects_its_output_list(
    #[case] response: &str,
    #[case] expected: Option<&str>,
) {
    let parsed = value(response);
    assert_eq!(
        response_output(&OutputResponse::ResponsesApi(&parsed)).as_deref(),
        expected
    );
}

#[rstest]
#[case::full(
    r#"{"type": "message", "role": "assistant", "content": [{"type": "text", "text": "hi"}], "model": "x"}"#,
    Some(r#"[{"content":[{"text":"hi","type":"text"}],"role":"assistant"}]"#)
)]
#[case::role_defaults_to_assistant(
    r#"{"type": "message", "content": "hi"}"#,
    Some(r#"[{"content":"hi","role":"assistant"}]"#)
)]
#[case::content_defaults_to_null(
    r#"{"type": "message", "role": "user"}"#,
    Some(r#"[{"content":null,"role":"user"}]"#)
)]
#[case::explicit_null_content(
    r#"{"type": "message", "content": null}"#,
    Some(r#"[{"content":null,"role":"assistant"}]"#)
)]
#[case::another_type(r#"{"type": "error", "content": "hi"}"#, None)]
#[case::no_type(r#"{"role": "assistant", "content": "hi"}"#, None)]
#[case::null_role(r#"{"type": "message", "role": null}"#, None)]
#[case::numeric_role(r#"{"type": "message", "role": 3}"#, None)]
#[case::a_text(r#""message""#, None)]
#[case::a_null("null", None)]
#[case::a_list(r#"[{"type": "message"}]"#, None)]
fn any_other_response_is_read_as_an_anthropic_message(
    #[case] response: &str,
    #[case] expected: Option<&str>,
) {
    let parsed = value(response);
    assert_eq!(
        response_output(&OutputResponse::Other(&parsed)).as_deref(),
        expected
    );
}

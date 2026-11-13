use rstest::rstest;

use crate::sampling_convert::content::SamplingRole;
use crate::sampling_convert::response::{
    CreateMessageResult, CreateMessageResultWithTools, SamplingErrorData, SamplingResponse,
    StopReason, convert_response, parse_tool_arguments,
};

use super::fixtures::{choice, response, text, tool_use};

#[test]
fn a_text_response_becomes_a_sampling_message() {
    let response = response("gpt-4o", vec![choice(Some("hello"), None, Some("stop"))]);
    assert_eq!(
        convert_response(&response, "gpt-4o").expect("conversion"),
        SamplingResponse::Message(CreateMessageResult {
            role: SamplingRole::Assistant,
            content: text("hello"),
            model: String::from("gpt-4o"),
            stop_reason: StopReason::EndTurn,
        })
    );
}

#[test]
fn an_empty_response_model_falls_back_to_the_resolved_name() {
    let response = response("", vec![choice(Some("hello"), None, Some("stop"))]);
    let converted = convert_response(&response, "resolved-model").expect("conversion");
    let SamplingResponse::Message(result) = converted else {
        panic!("expected a text result");
    };
    assert_eq!(result.model, "resolved-model");
}

#[rstest]
#[case::tool_calls("tool_calls", StopReason::ToolUse)]
#[case::length("length", StopReason::MaxTokens)]
#[case::stop("stop", StopReason::EndTurn)]
#[case::content_filter("content_filter", StopReason::EndTurn)]
fn finish_reasons_map_to_stop_reasons(#[case] finish: &str, #[case] expected: StopReason) {
    let response = response("gpt-4o", vec![choice(Some("x"), None, Some(finish))]);
    let converted = convert_response(&response, "gpt-4o").expect("conversion");
    let SamplingResponse::Message(result) = converted else {
        panic!("expected a text result");
    };
    assert_eq!(result.stop_reason, expected);
}

#[test]
fn a_missing_finish_reason_maps_to_end_turn() {
    let response = response("gpt-4o", vec![choice(Some("x"), None, None)]);
    let converted = convert_response(&response, "gpt-4o").expect("conversion");
    let SamplingResponse::Message(result) = converted else {
        panic!("expected a text result");
    };
    assert_eq!(result.stop_reason, StopReason::EndTurn);
}

#[test]
fn tool_calls_become_tool_use_content_after_the_text() {
    let response = response(
        "gpt-4o",
        vec![choice(
            Some("calling"),
            Some(&[("call-9", "lookup", r#"{"q": "x"}"#)]),
            Some("tool_calls"),
        )],
    );
    assert_eq!(
        convert_response(&response, "gpt-4o").expect("conversion"),
        SamplingResponse::WithTools(CreateMessageResultWithTools {
            role: SamplingRole::Assistant,
            content: vec![
                text("calling"),
                tool_use("call-9", "lookup", r#"{"q": "x"}"#)
            ],
            model: String::from("gpt-4o"),
            stop_reason: StopReason::ToolUse,
        })
    );
}

#[test]
fn tool_calls_without_text_have_only_tool_use_blocks() {
    let response = response(
        "gpt-4o",
        vec![choice(
            None,
            Some(&[("call-9", "lookup", "{}")]),
            Some("tool_calls"),
        )],
    );
    assert_eq!(
        convert_response(&response, "gpt-4o").expect("conversion"),
        SamplingResponse::WithTools(CreateMessageResultWithTools {
            role: SamplingRole::Assistant,
            content: vec![tool_use("call-9", "lookup", "{}")],
            model: String::from("gpt-4o"),
            stop_reason: StopReason::ToolUse,
        })
    );
}

#[test]
fn empty_text_is_omitted_from_a_tool_call_result() {
    let response = response(
        "gpt-4o",
        vec![choice(
            Some(""),
            Some(&[("call-9", "lookup", "{}")]),
            Some("tool_calls"),
        )],
    );
    assert_eq!(
        convert_response(&response, "gpt-4o").expect("conversion"),
        SamplingResponse::WithTools(CreateMessageResultWithTools {
            role: SamplingRole::Assistant,
            content: vec![tool_use("call-9", "lookup", "{}")],
            model: String::from("gpt-4o"),
            stop_reason: StopReason::ToolUse,
        })
    );
}

#[test]
fn an_empty_tool_calls_list_becomes_a_text_message() {
    let response = response(
        "gpt-4o",
        vec![choice(Some("hello"), Some(&[]), Some("tool_calls"))],
    );
    assert_eq!(
        convert_response(&response, "gpt-4o").expect("conversion"),
        SamplingResponse::Message(CreateMessageResult {
            role: SamplingRole::Assistant,
            content: text("hello"),
            model: String::from("gpt-4o"),
            stop_reason: StopReason::ToolUse,
        })
    );
}

#[test]
fn several_tool_calls_keep_their_order() {
    let response = response(
        "gpt-4o",
        vec![choice(
            None,
            Some(&[
                ("call-1", "first", "{}"),
                ("call-2", "second", r#"{"n": 2}"#),
            ]),
            Some("tool_calls"),
        )],
    );
    assert_eq!(
        convert_response(&response, "gpt-4o").expect("conversion"),
        SamplingResponse::WithTools(CreateMessageResultWithTools {
            role: SamplingRole::Assistant,
            content: vec![
                tool_use("call-1", "first", "{}"),
                tool_use("call-2", "second", r#"{"n": 2}"#),
            ],
            model: String::from("gpt-4o"),
            stop_reason: StopReason::ToolUse,
        })
    );
}

#[test]
fn no_choices_is_an_error_data_with_code_minus_one() {
    let response = response("gpt-4o", Vec::new());
    assert_eq!(
        convert_response(&response, "gpt-4o").expect("conversion"),
        SamplingResponse::Error(SamplingErrorData {
            code: -1,
            message: String::from(
                "LLM returned no choices for model 'gpt-4o'. This may indicate content filtering or a provider-side error."
            ),
        })
    );
}

#[test]
fn a_json_object_parses_to_itself() {
    let arguments = parse_tool_arguments(r#"{"q": 1, "path": "a/b"}"#).expect("arguments");
    assert_eq!(arguments.as_ref(), r#"{"q": 1, "path": "a/b"}"#);
}

#[test]
fn invalid_json_is_wrapped_under_raw() {
    let arguments = parse_tool_arguments("not json").expect("arguments");
    assert_eq!(arguments.as_ref(), r#"{"raw":"not json"}"#);
}

#[test]
fn the_raw_fallback_escapes_the_argument_text() {
    let arguments = parse_tool_arguments("a\"b").expect("arguments");
    assert_eq!(arguments.as_ref(), r#"{"raw":"a\"b"}"#);
}

#[test]
fn a_json_non_object_is_refused_with_the_python_message() {
    let error = parse_tool_arguments("[1, 2]").expect_err("not an object");
    assert_eq!(
        error.to_string(),
        "tool call arguments: Input should be a valid dictionary"
    );
}

#[test]
fn a_json_null_is_refused_as_a_non_object() {
    parse_tool_arguments("null").expect_err("not an object");
}

#[test]
fn an_empty_object_parses() {
    let arguments = parse_tool_arguments("{}").expect("arguments");
    assert_eq!(arguments.as_ref(), "{}");
}

#[test]
fn bad_tool_arguments_fail_the_response_conversion() {
    let response = response(
        "gpt-4o",
        vec![choice(
            None,
            Some(&[("call-9", "lookup", "\"hello\"")]),
            Some("tool_calls"),
        )],
    );
    let error = convert_response(&response, "gpt-4o").expect_err("refused arguments");
    assert_eq!(
        error.to_string(),
        "tool call arguments: Input should be a valid dictionary"
    );
}

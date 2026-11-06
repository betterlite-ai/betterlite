use betterlite_guardrails::vigil_guard::vigil_guard_output::build_output;

use crate::support::{inputs, text, tool_call};

#[test]
fn unchanged_input_is_returned_verbatim() {
    let calls = vec![tool_call("old")];
    let mut inputs = inputs(
        Some(vec!["a".to_owned(), "b".to_owned()]),
        Some(calls.clone()),
    );
    inputs.images = Some(vec![text("image")]);
    inputs.tools = Some(vec![text("tool")]);
    inputs.structured_messages = Some(vec![text("stale message")]);
    inputs.model = Some("gpt-4o".to_owned());
    inputs.stream_holdback_chars = Some(vec![2]);
    let final_texts = vec!["a".to_owned(), "b".to_owned()];
    let result = build_output(&inputs, &final_texts, Some(&calls));
    assert_eq!(result, inputs);
}

#[test]
fn absent_texts_compare_equal_to_an_empty_list() {
    let inputs = inputs(None, None);
    let result = build_output(&inputs, &[], None);
    assert_eq!(result, inputs);
}

#[test]
fn sanitized_text_drops_structured_messages() {
    let mut inputs = inputs(Some(vec!["a".to_owned(), "b".to_owned()]), None);
    inputs.images = Some(vec![text("image")]);
    inputs.tools = Some(vec![text("tool")]);
    inputs.structured_messages = Some(vec![text("stale message")]);
    inputs.model = Some("gpt-4o".to_owned());
    inputs.stream_holdback_chars = Some(vec![2]);
    let final_texts = vec!["sanitized".to_owned(), "b".to_owned()];
    let result = build_output(&inputs, &final_texts, None);
    assert_eq!(result.texts, Some(final_texts));
    assert_eq!(result.images, inputs.images);
    assert_eq!(result.tools, inputs.tools);
    assert_eq!(result.structured_messages, None);
    assert_eq!(result.model, None);
    assert_eq!(result.stream_holdback_chars, None);
}

#[test]
fn unchanged_tool_calls_are_omitted_after_text_sanitization() {
    let calls = vec![tool_call("old")];
    let inputs = inputs(Some(vec!["a".to_owned()]), Some(calls.clone()));
    let final_texts = vec!["sanitized".to_owned()];
    let result = build_output(&inputs, &final_texts, Some(&calls));
    assert_eq!(result.texts, Some(final_texts));
    assert_eq!(result.tool_calls, None);
}

#[test]
fn changed_tool_calls_are_returned() {
    let inputs = inputs(Some(vec!["a".to_owned()]), Some(vec![tool_call("old")]));
    let final_calls = vec![tool_call("new")];
    let final_texts = vec!["a".to_owned()];
    let result = build_output(&inputs, &final_texts, Some(&final_calls));
    assert_eq!(result.texts, Some(final_texts));
    assert_eq!(result.tool_calls, Some(final_calls));
}

#[test]
fn missing_tool_calls_stay_missing_after_text_sanitization() {
    let inputs = inputs(Some(vec!["a".to_owned()]), None);
    let final_texts = vec!["sanitized".to_owned()];
    let result = build_output(&inputs, &final_texts, None);
    assert_eq!(result.texts, Some(final_texts));
    assert_eq!(result.tool_calls, None);
    assert_eq!(result.structured_messages, None);
}

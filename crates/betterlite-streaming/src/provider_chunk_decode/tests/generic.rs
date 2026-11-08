#[test]
fn generic_chunk_carries_text_and_fields() {
    let mut decode_state = state("gpt-4");
    let chunk = generic_chunk(
        r#"{"text": "hi", "tool_use": null, "is_finished": false, "finish_reason": "", "usage": null, "provider_specific_fields": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some(""));
    assert!(decoded.usage.is_none());
}

#[test]
fn generic_json_chunk_takes_the_generic_branch_for_a_builtin_provider() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"text": "hello", "is_finished": true, "finish_reason": "stop", "usage": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hello");
    assert!(decoded.is_finished);
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        decode_state.intermittent_finish_reason.as_deref(),
        Some("stop")
    );
}

#[test]
fn generic_tool_use_becomes_a_one_element_tool_calls_list() {
    let mut decode_state = state("gpt-4");
    let chunk = generic_chunk(
        r#"{"text": "", "tool_use": {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}, "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    let calls = decoded.tool_calls.as_ref().expect("the chunk has calls");
    let items = as_array(calls).expect("tool calls are a list");
    assert_eq!(items.len(), 1);
    assert_eq!(decoded.tool_use, items.first().cloned());
}

#[test]
fn generic_usage_and_provider_fields_are_carried() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"text": "x", "is_finished": false, "finish_reason": "", "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}, "provider_specific_fields": {"trace": "t1"}}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.usage, Some(usage(3, 4, 7)));
    let fields = decoded
        .provider_specific_fields
        .expect("provider fields are carried");
    assert_eq!(fields.get("trace").and_then(JsonValue::as_str), Some("t1"));
}

#[test]
fn generic_stop_iteration_after_a_finish_reason_without_content() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk =
        generic_chunk(r#"{"text": "", "is_finished": false, "finish_reason": "", "usage": null}"#);
    let error = decode_error("openai", &chunk, &mut decode_state);
    assert!(matches!(error, ProviderChunkError::StopIteration));
}

#[test]
fn generic_provider_fields_keep_an_empty_chunk_after_a_finish_reason() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk = json_chunk(
        r#"{"text": "", "is_finished": false, "finish_reason": "", "usage": null, "provider_specific_fields": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "");
}

#[test]
fn generic_content_after_a_finish_reason_is_accepted() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk = generic_chunk(
        r#"{"text": "more", "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "more");
}

#[test]
fn generic_custom_provider_accepts_a_generic_chunk() {
    let mut decode_state = custom_state("my-model");
    let chunk = generic_chunk(
        r#"{"text": "custom", "is_finished": true, "finish_reason": "stop", "usage": null}"#,
    );
    let decoded = decode("my-provider", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "custom");
    assert!(decoded.is_finished);
}

#[test]
fn generic_custom_provider_rejects_a_text_chunk() {
    let mut decode_state = custom_state("custom-model");
    let error = decode_error("custom-provider", &text("plain"), &mut decode_state);
    assert_eq!(
        error.to_string(),
        "the provider stream did not send a generic streaming chunk"
    );
}

#[test]
fn generic_custom_content_after_a_finish_reason_is_accepted() {
    let mut decode_state = custom_state("custom-model");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk = json_chunk(r#"{"id":"c1","choices":[{"index":0,"delta":{"content":"hi"}}]}"#);
    let decoded = decode("custom-provider", &chunk, &mut decode_state);
    assert!(decoded.early_return);
    assert!(decoded.original.is_some());
}

#[test]
fn generic_custom_empty_tool_calls_stop_the_stream() {
    let mut decode_state = custom_state("custom-model");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk =
        json_chunk(r#"{"id":"c1","choices":[{"index":0,"delta":{"content":"","tool_calls":[]}}]}"#);
    let error = decode_error("custom-provider", &chunk, &mut decode_state);
    assert!(matches!(error, ProviderChunkError::StopIteration));
}

#[test]
fn generic_tool_use_after_a_finish_reason_is_accepted() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk = generic_chunk(
        r#"{"text": "", "tool_use": {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}, "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(decoded.tool_use.is_some());
}

#[test]
fn generic_custom_tool_calls_after_a_finish_reason_are_accepted() {
    let mut decode_state = custom_state("custom-model");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let chunk = json_chunk(
        r#"{"id":"c1","choices":[{"index":0,"delta":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}}]}"#,
    );
    let decoded = decode("custom-provider", &chunk, &mut decode_state);
    assert!(decoded.early_return);
}

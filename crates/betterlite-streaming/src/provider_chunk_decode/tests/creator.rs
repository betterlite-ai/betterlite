#[test]
fn creator_fills_the_role_of_a_plain_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#);
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let choice = first_choice(&created_chunk);
    assert_eq!(choice.delta.content.as_deref(), Some("hi"));
    assert_eq!(choice.delta.role.as_deref(), Some("assistant"));
    assert_eq!(created_chunk.model, "gpt-4");
    assert_eq!(created_chunk.id, "c1");
}

#[test]
fn creator_drops_the_role_after_the_first_chunk() {
    let mut decode_state = state("gpt-4");
    let first = json_chunk(r#"{"id": "c1", "choices": [{"delta": {"content": "a"}}]}"#);
    created("openai", &mut decode_state, &first).expect("the first chunk is emitted");
    let second = json_chunk(r#"{"id": "c1", "choices": [{"delta": {"content": "b"}}]}"#);
    let created_chunk =
        created("openai", &mut decode_state, &second).expect("the second is emitted");
    assert_eq!(first_choice(&created_chunk).delta.role, None);
    assert!(decode_state.sent_first_chunk);
}

#[test]
fn creator_early_return_passes_the_custom_chunk_through() {
    let mut decode_state = custom_state("my-model");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": "stop"}]}"#,
    );
    let created_chunk =
        created("my-provider", &mut decode_state, &chunk).expect("the chunk passes");
    let choice = first_choice(&created_chunk);
    assert_eq!(choice.delta.content.as_deref(), Some("hi"));
    assert_eq!(choice.finish_reason, None);
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn creator_custom_finish_only_chunk_is_skipped() {
    let mut decode_state = custom_state("my-model");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}"#,
    );
    let emitted = created("my-provider", &mut decode_state, &chunk);
    assert!(emitted.is_none());
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn creator_tool_use_becomes_function_call_when_functions_were_sent() {
    let mut decode_state = state("gpt-4");
    decode_state.options.is_function_call = true;
    let chunk = generic_chunk(
        r#"{"text": "", "tool_use": {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}, "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let delta = &first_choice(&created_chunk).delta;
    assert!(delta.tool_calls.is_none());
    let function = delta
        .function_call
        .as_ref()
        .expect("the call became a function call");
    assert_eq!(
        as_object(function)
            .and_then(|fields| fields.get("name"))
            .and_then(JsonValue::as_str),
        Some("f")
    );
}

#[test]
fn creator_tool_use_stays_tool_calls_without_functions() {
    let mut decode_state = state("gpt-4");
    let chunk = generic_chunk(
        r#"{"text": "", "tool_use": {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}, "is_finished": false, "finish_reason": "", "usage": null}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let delta = &first_choice(&created_chunk).delta;
    assert!(delta.function_call.is_none());
    assert!(delta.tool_calls.is_some());
}

#[test]
fn creator_usage_only_chunk_is_recorded_without_send_usage() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [], "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}"#,
    );
    let mut shaping = shaper("openai");
    let emitted = chunk_creator(&mut shaping, &mut decode_state, &chunk).expect("the chunk shapes");
    assert!(emitted.is_none());
    assert_eq!(shaping.chunks.len(), 1);
    assert!(
        shaping
            .chunks
            .first()
            .expect("the recorded chunk")
            .choices
            .is_empty()
    );
}

#[test]
fn creator_usage_only_chunk_is_emitted_with_send_usage() {
    let mut decode_state = state("gpt-4");
    decode_state.options.send_stream_usage = true;
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [], "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("the chunk is emitted");
    assert!(created_chunk.choices.is_empty());
    assert_eq!(created_chunk.usage, Some(usage(2, 3, 5)));
}

#[test]
fn creator_azure_model_reaches_the_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "model": "azure-deployment", "choices": [{"delta": {"content": "hi"}}]}"#,
    );
    let created_chunk = created("azure", &mut decode_state, &chunk).expect("a chunk is emitted");
    assert_eq!(created_chunk.model, "azure-deployment");
}

#[test]
fn creator_generic_provider_fields_land_on_the_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"text": "hi", "is_finished": false, "finish_reason": "", "usage": null, "provider_specific_fields": {"provider_specific_fields": {"x": 1}}}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let fields = created_chunk
        .provider_specific_fields
        .expect("the provider fields are set");
    assert!(fields.contains_key("x"));
}

#[test]
fn creator_system_fingerprint_comes_from_the_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"system_fingerprint": "fp-2", "choices": [{"delta": {"content": "hi"}}]}"#);
    let mut shaping = shaper("openai");
    let created_chunk = chunk_creator(&mut shaping, &mut decode_state, &chunk)
        .expect("the chunk shapes")
        .expect("a chunk is emitted");
    assert_eq!(created_chunk.system_fingerprint.as_deref(), Some("fp-2"));
    assert_eq!(shaping.system_fingerprint.as_deref(), Some("fp-2"));
}

#[test]
fn creator_finish_only_chunk_is_the_terminal_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"id": "c1", "choices": [{"delta": {}, "finish_reason": "length"}]}"#);
    let mut shaping = shaper("openai");
    let created_chunk = chunk_creator(&mut shaping, &mut decode_state, &chunk)
        .expect("the chunk shapes")
        .expect("a chunk is emitted");
    assert_eq!(
        first_choice(&created_chunk).finish_reason.as_deref(),
        Some("length")
    );
    assert!(shaping.progress.sent_last_chunk);
}

#[test]
fn creator_repetition_limit_is_honoured() {
    let mut decode_state = state("gpt-4");
    decode_state.repeated_chunk_limit = 2;
    let mut shaping = shaper("openai");
    let first = json_chunk(r#"{"id": "c1", "choices": [{"delta": {"content": "repeat"}}]}"#);
    chunk_creator(&mut shaping, &mut decode_state, &first).expect("the first chunk shapes");
    let second = json_chunk(r#"{"id": "c1", "choices": [{"delta": {"content": "repeat"}}]}"#);
    let error =
        chunk_creator(&mut shaping, &mut decode_state, &second).expect_err("the repetition fails");
    assert!(matches!(error, ProviderChunkError::Shape { .. }));
}

#[test]
fn creator_ignores_the_id_of_a_non_openai_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"{"output": "hi", "id": "chunk-id"}"#);
    let mut shaping = shaper("replicate");
    shaping.response_id = Some("chatcmpl-state".to_owned());
    let created_chunk = chunk_creator(&mut shaping, &mut decode_state, &chunk)
        .expect("the chunk shapes")
        .expect("a chunk is emitted");
    assert_eq!(created_chunk.id, "chatcmpl-state");
}

#[test]
fn creator_empty_tool_calls_are_not_used() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "", "tool_calls": []}}]}"#,
    );
    let mut shaping = shaper("cached_response");
    let created_chunk = chunk_creator(&mut shaping, &mut decode_state, &chunk)
        .expect("the chunk shapes")
        .expect("a chunk is emitted");
    assert!(!shaping.tool_call);
    let calls = first_choice(&created_chunk)
        .delta
        .tool_calls
        .as_ref()
        .expect("the calls are kept");
    assert!(as_array(calls).expect("the calls are a list").is_empty());
}

#[test]
fn creator_keeps_no_usage_for_a_non_openai_chunk_with_a_usage_field() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"output": "hi", "usage": {"prompt_tokens": 9, "completion_tokens": 9, "total_tokens": 18}}"#,
    );
    let created_chunk =
        created("replicate", &mut decode_state, &chunk).expect("a chunk is emitted");
    assert!(created_chunk.usage.is_none());
}

#[test]
fn creator_hidden_params_include_the_provider_model() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "model": "provider-model", "choices": [{"delta": {"content": "hi"}}]}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    assert_eq!(
        created_chunk
            .hidden_params
            .get("provider_response_model")
            .and_then(JsonValue::as_str),
        Some("provider-model")
    );
}

#[test]
fn creator_carries_hidden_provider_fields() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"delta": {"content": "hi"}}], "hidden_params": {"provider_specific_fields": {"trace": "t1"}}}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    assert!(
        created_chunk
            .hidden_params
            .contains_key("provider_specific_fields")
    );
}

#[test]
fn creator_first_role_only_chunk_is_emitted() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"id": "c1", "choices": [{"index": 0, "delta": {"role": "assistant"}}]}"#);
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    assert_eq!(
        first_choice(&created_chunk).delta.role.as_deref(),
        Some("assistant")
    );
}

#[test]
fn creator_tool_call_delta_is_emitted() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}}]}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let calls = first_choice(&created_chunk)
        .delta
        .tool_calls
        .as_ref()
        .expect("the calls are kept");
    assert_eq!(as_array(calls).expect("the calls are a list").len(), 1);
}

#[test]
fn creator_provider_fields_set_the_chunk_identity() {
    let mut decode_state = state("gpt-4");
    let chunk = provider_field_chunk();
    let created_chunk = chunk_creator(&mut shaper("openai"), &mut decode_state, &chunk)
        .expect("the chunk shapes")
        .expect("a chunk is emitted");
    assert_eq!(created_chunk.id, "provider-id");
    assert_eq!(created_chunk.object, "provider.object");
    assert_eq!(created_chunk.model, "provider-model");
    assert_eq!(created_chunk.created, 1234);
    assert_eq!(
        created_chunk.system_fingerprint.as_deref(),
        Some("provider-fp")
    );
}

#[test]
fn creator_provider_fields_set_the_structured_fields() {
    let mut decode_state = state("gpt-4");
    let chunk = provider_field_chunk();
    let created_chunk = chunk_creator(&mut shaper("openai"), &mut decode_state, &chunk)
        .expect("the chunk shapes")
        .expect("a chunk is emitted");
    assert_eq!(created_chunk.usage, Some(usage(1, 2, 3)));
    assert_eq!(
        created_chunk
            .provider_specific_fields
            .as_ref()
            .and_then(|fields| fields.get("nested"))
            .and_then(JsonValue::as_i64),
        Some(1)
    );
    assert_eq!(
        created_chunk
            .hidden_params
            .get("marker")
            .and_then(JsonValue::as_str),
        Some("h")
    );
    assert_eq!(created_chunk.choices.len(), 1);
    assert_eq!(first_choice(&created_chunk).index, 3);
    assert_eq!(
        created_chunk
            .extra
            .get("unknown_key")
            .and_then(JsonValue::as_str),
        Some("extra")
    );
}

#[test]
fn creator_function_call_arguments_default_to_empty() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"function_call": {"name": "f"}}}]}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let function = first_choice(&created_chunk)
        .delta
        .function_call
        .as_ref()
        .expect("the function call is kept");
    assert_eq!(
        as_object(function).and_then(|fields| fields.get("arguments")),
        Some(&JsonValue::String(String::new()))
    );
}

#[test]
fn creator_tool_call_type_becomes_function() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"tool_calls": [{"id": "call_1", "type": null, "function": {"name": "f", "arguments": "{}"}}]}}]}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let call = first_tool_call(&created_chunk);
    assert_eq!(
        as_object(&call)
            .and_then(|fields| fields.get("type"))
            .and_then(JsonValue::as_str),
        Some("function")
    );
}

#[test]
fn creator_tool_without_function_keeps_its_type() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"tool_calls": [{"id": "call_1", "type": null}]}}]}"#,
    );
    let created_chunk = created("openai", &mut decode_state, &chunk).expect("a chunk is emitted");
    let call = first_tool_call(&created_chunk);
    assert_eq!(
        as_object(&call).and_then(|fields| fields.get("type")),
        Some(&JsonValue::Null)
    );
}

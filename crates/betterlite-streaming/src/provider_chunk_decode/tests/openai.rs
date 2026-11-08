#[test]
fn openai_keeps_usage_from_the_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "hi"}}], "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.usage, Some(usage(2, 3, 5)));
    assert_eq!(decoded.text, "hi");
}

#[test]
fn openai_takes_the_model_from_an_azure_chunk() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "model": "azure-deployment", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#,
    );
    let decoded = decode("azure", &chunk, &mut decode_state);
    assert_eq!(decode_state.model, "azure-deployment");
    assert_eq!(decoded.text, "hi");
}

#[test]
fn openai_azure_ai_reads_the_model_too() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"model": "ai-deployment", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#,
    );
    decode("azure_ai", &chunk, &mut decode_state);
    assert_eq!(decode_state.model, "ai-deployment");
}

#[test]
fn openai_azure_empty_model_keeps_the_model() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"model": "", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#);
    decode("azure", &chunk, &mut decode_state);
    assert_eq!(decode_state.model, "gpt-4");
}

#[test]
fn openai_plain_provider_keeps_the_model() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"model": "other", "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#);
    decode("openai", &chunk, &mut decode_state);
    assert_eq!(decode_state.model, "gpt-4");
}

#[test]
fn openai_finish_reason_sets_the_received_reason() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"choices": [{"delta": {"content": "hi"}, "finish_reason": "stop"}]}"#);
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        decode_state.intermittent_finish_reason.as_deref(),
        Some("stop")
    );
}

#[test]
fn openai_empty_finish_reason_is_no_finish() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"{"choices": [{"delta": {"content": "hi"}, "finish_reason": ""}]}"#);
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert!(!decoded.is_finished);
    assert!(decoded.finish_reason.is_none());
    assert!(decode_state.intermittent_finish_reason.is_none());
}

#[test]
fn openai_clears_the_intermittent_reason_on_a_chunk_without_one() {
    let mut decode_state = state("gpt-4");
    decode_state.intermittent_finish_reason = Some("length".to_owned());
    let chunk = json_chunk(r#"{"choices": [{"delta": {"content": "hi"}}]}"#);
    decode("openai", &chunk, &mut decode_state);
    assert!(decode_state.intermittent_finish_reason.is_none());
}

#[test]
fn openai_hidden_params_is_finished_overrides_the_flag() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"choices": [{"delta": {"content": "hi"}, "finish_reason": "stop"}], "hidden_params": {"is_finished": false}}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert!(!decoded.is_finished);
    assert!(decode_state.received_finish_reason.is_none());
}

#[test]
fn openai_logprobs_are_carried() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"choices": [{"delta": {"content": "hi"}, "logprobs": {"tokens": ["hi"]}}]}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert!(decoded.logprobs.is_some());
}

#[test]
fn openai_system_fingerprint_is_recorded() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"system_fingerprint": "fp-1", "choices": [{"delta": {"content": "hi"}}]}"#);
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.system_fingerprint, Some(Some("fp-1".to_owned())));
    assert_eq!(
        decoded.response_system_fingerprint,
        Some(Some("fp-1".to_owned()))
    );
}

#[test]
fn openai_missing_choices_gives_empty_text() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"{"id": "c1"}"#);
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(!decoded.is_finished);
    assert!(decoded.original.is_some());
}

#[test]
fn openai_error_finish_reason_fails() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"choices": [{"delta": {"content": "hi"}, "finish_reason": "error"}]}"#);
    let error = decode_error("openai", &chunk, &mut decode_state);
    assert!(matches!(error, ProviderChunkError::StreamingFailure { .. }));
}

#[test]
fn openai_vertex_openai_shaped_chunk_keeps_the_openai_reader() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": "stop"}]}"#,
    );
    let decoded = decode("vertex_ai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
}

#[test]
fn openai_null_delta_is_an_empty_delta() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": null, "finish_reason": "stop"}]}"#,
    );
    let decoded = decode("openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "");
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn openai_cached_response_reads_the_choice() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "cached"}, "finish_reason": "stop"}]}"#,
    );
    let decoded = decode("cached_response", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "cached");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert!(decoded.original.is_some());
    assert!(decoded.usage.is_none());
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn openai_cached_response_keeps_the_chunk_usage() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"id": "c1", "choices": [{"index": 0, "delta": {"content": "x"}}], "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}}"#,
    );
    let decoded = decode("cached_response", &chunk, &mut decode_state);
    assert!(decoded.usage.is_none());
    let created_chunk =
        created("cached_response", &mut decode_state, &chunk).expect("a chunk is emitted");
    assert_eq!(created_chunk.usage, Some(usage(1, 2, 3)));
}

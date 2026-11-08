#[test]
fn text_providers_predibase_reads_the_token_text() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data:{"token": {"text": "hi"}}"#);
    let decoded = decode("predibase", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
}

#[test]
fn text_providers_predibase_takes_the_details_finish_reason() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data:{"details": {"finish_reason": "length"}}"#);
    let decoded = decode("predibase", &chunk, &mut decode_state);
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("length"));
    assert_eq!(
        decode_state.received_finish_reason.as_deref(),
        Some("length")
    );
}

#[test]
fn text_providers_predibase_generated_text_ends_the_stream() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data:{"generated_text": "full answer"}"#);
    let decoded = decode("predibase", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_predibase_reports_an_error_field() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data:{"error": "boom"}"#);
    let error = decode_error("predibase", &chunk, &mut decode_state);
    assert_eq!(error.to_string(), "boom");
}

#[test]
fn text_providers_predibase_rejects_a_plain_error_line() {
    let mut decode_state = state("gpt-4");
    let error = decode_error("predibase", &text("an error happened"), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::RejectedResponse { .. }));
}

#[test]
fn text_providers_predibase_plain_line_is_empty() {
    let mut decode_state = state("gpt-4");
    let decoded = decode("predibase", &text("plain"), &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(!decoded.is_finished);
}

#[test]
fn text_providers_predibase_accepts_bytes() {
    let mut decode_state = state("gpt-4");
    let chunk = ProviderChunk::Bytes(br#"data:{"token": {"text": "hi"}}"#.to_vec());
    let decoded = decode("predibase", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
}

#[test]
fn text_providers_nlp_cloud_reads_generated_text() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"{"generated_text": "hello"}"#);
    let decoded = decode("nlp_cloud", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hello");
}

#[test]
fn text_providers_nlp_cloud_done_ends_the_stream() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"{"generated_text": "hello [DONE]"}"#);
    let decoded = decode("nlp_cloud", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hello ");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_nlp_cloud_dolphin_strips_the_complete_response() {
    let mut decode_state = state("dolphin-2.2");
    let first = decode("nlp_cloud", &text(r#""Hello"#), &mut decode_state);
    assert_eq!(first.text, "\"Hello");
    assert_eq!(decode_state.complete_response, "\"Hello");
    let second = decode("nlp_cloud", &text(r#""Hello world""#), &mut decode_state);
    assert_eq!(second.text, " world\"");
    assert_eq!(decode_state.complete_response, "\"Hello world\"");
}

#[test]
fn text_providers_nlp_cloud_after_first_chunk_recovers_with_stop() {
    let mut decode_state = state("gpt-4");
    decode_state.sent_first_chunk = true;
    let decoded = decode("nlp_cloud", &text("not json"), &mut decode_state);
    assert_eq!(decoded.text, "");
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_aleph_alpha_takes_the_first_completion() {
    let mut decode_state = state("gpt-4");
    let chunk = ProviderChunk::Bytes(br#"{"completions": [{"completion": "hi"}]}"#.to_vec());
    let decoded = decode("aleph_alpha", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_aleph_alpha_missing_completion_is_rejected() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "aleph_alpha",
        &text(r#"{"completions": []}"#),
        &mut decode_state,
    );
    assert!(matches!(error, ProviderChunkError::RejectedResponse { .. }));
}

#[test]
fn text_providers_replicate_reads_output() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "replicate",
        &json_chunk(r#"{"output": "hi"}"#),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
}

#[test]
fn text_providers_replicate_succeeded_status_finishes() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "replicate",
        &json_chunk(r#"{"status": "succeeded"}"#),
        &mut decode_state,
    );
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_replicate_model_name_selects_the_reader() {
    let mut decode_state = state("replicate");
    let decoded = decode(
        "openai",
        &json_chunk(r#"{"status": "processing", "output": "hi"}"#),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "hi");
}

#[test]
fn text_providers_replicate_error_is_rejected() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "replicate",
        &json_chunk(r#"{"error": "bad"}"#),
        &mut decode_state,
    );
    assert!(matches!(error, ProviderChunkError::RejectedResponse { .. }));
}

#[test]
fn text_providers_replicate_rejects_a_non_text_output() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "replicate",
        &json_chunk(r#"{"output": [1, 2]}"#),
        &mut decode_state,
    );
    assert!(matches!(
        error,
        ProviderChunkError::InvalidField { field: "output" }
    ));
}

#[test]
fn text_providers_vllm_takes_the_first_output_text() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"[{"outputs": [{"text": "hi"}]}]"#);
    let decoded = decode("vllm", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
}

#[test]
fn text_providers_vllm_missing_output_is_an_error() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "vllm",
        &json_chunk(r#"[{"outputs": []}]"#),
        &mut decode_state,
    );
    assert!(matches!(error, ProviderChunkError::MissingField { .. }));
}

#[test]
fn text_providers_vllm_rejects_a_non_list_chunk() {
    let mut decode_state = state("gpt-4");
    let error = decode_error("vllm", &json_chunk(r#"{"outputs": []}"#), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::UnexpectedChunk { .. }));
}

#[test]
fn text_providers_petals_slices_thirty_characters() {
    let mut decode_state = state("gpt-4");
    decode_state.completion_stream = Some("a".repeat(40));
    let decoded = decode("petals", &text("ignored"), &mut decode_state);
    assert_eq!(decoded.text, "a".repeat(30));
    assert_eq!(
        decode_state.completion_stream.as_deref(),
        Some("a".repeat(10).as_str())
    );
}

#[test]
fn text_providers_petals_slices_by_characters_not_bytes() {
    let mut decode_state = state("gpt-4");
    decode_state.completion_stream = Some("\u{e9}".repeat(31));
    let decoded = decode("petals", &text("ignored"), &mut decode_state);
    assert_eq!(decoded.text, "\u{e9}".repeat(30));
    assert_eq!(decode_state.completion_stream.as_deref(), Some("\u{e9}"));
}

#[test]
fn text_providers_petals_empty_stream_sets_stop() {
    let mut decode_state = state("gpt-4");
    let decoded = decode("petals", &text("ignored"), &mut decode_state);
    assert_eq!(decoded.text, "");
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_petals_stops_once_finished() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let error = decode_error("petals", &text("ignored"), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::StopIteration));
}

#[test]
fn text_providers_triton_reads_the_object() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "triton",
        &json_chunk(r#"{"text_output": "hi", "stop_reason": "stop", "is_finished": true}"#),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "hi");
    assert!(decoded.is_finished);
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_triton_reads_an_sse_text_line() {
    let mut decode_state = state("gpt-4");
    let chunk = text(r#"data: {"text_output": "hi", "is_finished": false}"#);
    let decoded = decode("triton", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
}

#[test]
fn text_providers_triton_line_without_text_output_is_empty() {
    let mut decode_state = state("gpt-4");
    let decoded = decode("triton", &text("nothing here"), &mut decode_state);
    assert_eq!(decoded.text, "");
    assert!(!decoded.is_finished);
}

#[test]
fn text_providers_text_completion_openai_keeps_text_and_usage() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"choices": [{"text": "hi", "finish_reason": "stop"}], "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5, "cost": 1}}"#,
    );
    let decoded = decode("text-completion-openai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert_eq!(decoded.usage, Some(usage(2, 3, 5)));
}

#[test]
fn text_providers_azure_text_reads_choices() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(r#"{"choices": [{"text": "hi", "finish_reason": "length"}]}"#);
    let decoded = decode("azure_text", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert_eq!(decoded.finish_reason.as_deref(), Some("length"));
    assert!(decoded.usage.is_none());
}

#[test]
fn text_providers_azure_text_ignores_a_non_object_chunk() {
    let mut decode_state = state("gpt-4");
    let decoded = decode("azure_text", &text("ignored"), &mut decode_state);
    assert_eq!(decoded.text, "");
}

#[test]
fn text_providers_codestral_parses_the_delta_content() {
    let mut decode_state = state("gpt-4");
    let chunk = text(
        r#"data: {"id": "c", "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": null}]}"#,
    );
    let decoded = decode("text-completion-codestral", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
    assert!(decoded.original.is_some());
}

#[test]
fn text_providers_codestral_done_is_empty() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "text-completion-codestral",
        &text("data: [DONE]"),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "");
    assert!(decoded.original.is_none());
}

#[test]
fn text_providers_codestral_invalid_json_is_empty() {
    let mut decode_state = state("gpt-4");
    let decoded = decode(
        "text-completion-codestral",
        &text("data: {not json"),
        &mut decode_state,
    );
    assert_eq!(decoded.text, "");
}

#[test]
fn text_providers_codestral_finish_reason_finishes() {
    let mut decode_state = state("gpt-4");
    let chunk = text(
        r#"{"choices": [{"delta": {"content": ""}, "finish_reason": "stop", "logprobs": {"a": 1}}]}"#,
    );
    let decoded = decode("text-completion-codestral", &chunk, &mut decode_state);
    assert!(decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
    assert!(decoded.logprobs.is_some());
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn text_providers_triton_is_finished_false() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"text_output": "hi", "is_finished": false, "stop_reason": "length"}"#);
    let decoded = decode("triton", &chunk, &mut decode_state);
    assert!(!decoded.is_finished);
    assert_eq!(decoded.finish_reason.as_deref(), Some("length"));
    assert!(decode_state.received_finish_reason.is_none());
}

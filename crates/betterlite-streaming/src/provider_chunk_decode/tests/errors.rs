#[test]
fn errors_nlp_cloud_message_matches_litellm() {
    let mut decode_state = state("gpt-4");
    let error = decode_error("nlp_cloud", &text("not json"), &mut decode_state);
    assert_eq!(
        error.to_string(),
        "Unable to parse response. Original response: not json"
    );
}

#[test]
fn errors_nlp_cloud_unknown_stream_before_the_first_chunk() {
    let mut decode_state = state("gpt-4");
    let error = decode_error("nlp_cloud", &text("not json"), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::UnknownStream));
}

#[test]
fn errors_nlp_cloud_keeps_the_error_after_a_finish_reason() {
    let mut decode_state = state("gpt-4");
    decode_state.received_finish_reason = Some("stop".to_owned());
    let error = decode_error("nlp_cloud", &text("not json"), &mut decode_state);
    assert!(matches!(error, ProviderChunkError::ParseResponse { .. }));
}

#[test]
fn errors_vertex_blocked_message() {
    let mut decode_state = state("gpt-4");
    let chunk =
        json_chunk(r#"{"candidates": [{"content": {"parts": []}, "finish_reason": "SAFETY"}]}"#);
    let error = decode_error("vertex_ai", &chunk, &mut decode_state);
    assert_eq!(error.to_string(), "The response was blocked by VertexAI.");
}

#[test]
fn errors_vertex_maps_the_finish_reason() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"candidates": [{"content": {"parts": [{"text": "hi"}]}, "finish_reason": "STOP"}]}"#,
    );
    let decoded = decode("vertex_ai", &chunk, &mut decode_state);
    assert_eq!(decoded.text, "hi");
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[test]
fn errors_vertex_unspecified_finish_reason_is_ignored() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"candidates": [{"content": {"parts": [{"text": "hi"}]}, "finish_reason": "FINISH_REASON_UNSPECIFIED"}]}"#,
    );
    decode("vertex_ai", &chunk, &mut decode_state);
    assert!(decode_state.received_finish_reason.is_none());
}

#[test]
fn errors_predibase_error_message() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "predibase",
        &text(r#"data:{"error": "boom"}"#),
        &mut decode_state,
    );
    assert_eq!(error.to_string(), "boom");
}

#[test]
fn errors_replicate_error_message_contains_the_chunk() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "replicate",
        &json_chunk(r#"{"error": "bad"}"#),
        &mut decode_state,
    );
    assert_eq!(
        error.to_string(),
        r#"Unable to parse response. Original response: {"error":"bad"}"#
    );
}

#[test]
fn errors_codestral_rejects_a_non_text_chunk() {
    let mut decode_state = state("gpt-4");
    let error = decode_error(
        "text-completion-codestral",
        &json_chunk(r#"{"choices": []}"#),
        &mut decode_state,
    );
    assert!(matches!(error, ProviderChunkError::NotAString { .. }));
}

#[test]
fn errors_invalid_utf8_bytes_are_refused() {
    let mut decode_state = state("gpt-4");
    let chunk = ProviderChunk::Bytes(vec![0xff, 0xfe]);
    let error = decode_error("predibase", &chunk, &mut decode_state);
    assert!(matches!(error, ProviderChunkError::InvalidUtf8 { .. }));
}

#[test]
fn errors_vertex_function_call_is_built() {
    let mut decode_state = state("gpt-4");
    let chunk = json_chunk(
        r#"{"candidates": [{"content": {"parts": [{"function_call": {"name": "lookup", "args": {"where": "x"}}}]}, "finish_reason": "STOP"}]}"#,
    );
    let decoded = decode("vertex_ai", &chunk, &mut decode_state);
    let calls = decoded.tool_calls.as_ref().expect("a call is built");
    let items = as_array(calls).expect("the calls are a list");
    let call = as_object(items.first().expect("one call")).expect("the call is an object");
    let identifier = call.get("id").and_then(JsonValue::as_str);
    assert!(identifier.is_some_and(|value| value.starts_with("call_")));
    assert_eq!(
        call.get("type").and_then(JsonValue::as_str),
        Some("function")
    );
    let function =
        as_object(call.get("function").expect("a function")).expect("the function is an object");
    assert_eq!(
        function.get("name").and_then(JsonValue::as_str),
        Some("lookup")
    );
    assert_eq!(
        function.get("arguments").and_then(JsonValue::as_str),
        Some(r#"{"where":"x"}"#)
    );
    assert!(
        decoded
            .original
            .as_ref()
            .is_some_and(|original| !original.choices.is_empty())
    );
    assert_eq!(decode_state.received_finish_reason.as_deref(), Some("stop"));
}

#[rstest]
#[case::content(Some("x"), true)]
#[case::empty_content(Some(""), false)]
#[case::absent(None::<&str>, false)]
fn non_empty_reads_completion_content(#[case] content: Option<&str>, #[case] expected: bool) {
    let shaper = shaper("gpt-4", Some("openai"));
    let mut completion = empty_delta();
    completion.content = content.map(str::to_owned);
    let response = chunk("c", 1);
    assert_eq!(
        is_chunk_non_empty(
            &shaper,
            &completion,
            &response,
            &ChunkResponseObject::default()
        ),
        expected
    );
}

#[rstest]
#[case::tool_calls(r#"{"tool_calls": [{"id": "t"}]}"#, true)]
#[case::empty_tool_calls(r#"{"tool_calls": []}"#, false)]
#[case::function_call(r#"{"function_call": {"name": "f"}}"#, true)]
#[case::reasoning(r#"{"reasoning_content": "r"}"#, true)]
#[case::reasoning_items(r#"{"reasoning_items": [{"id": "r"}]}"#, true)]
#[case::provider_fields(r#"{"provider_specific_fields": {"a": 1}}"#, true)]
#[case::annotations(r#"{"annotations": [{"type": "url_citation"}]}"#, true)]
#[case::audio(r#"{"audio": {"data": "x"}}"#, false)]
#[case::images(r#"{"images": [{"url": "u"}]}"#, false)]
fn non_empty_reads_the_completion_object_and_delta(#[case] source: &str, #[case] expected: bool) {
    let shaper = shaper("gpt-4", Some("openai"));
    let completion: StreamDelta = parse(source);
    let mut response = chunk("c", 1);
    response.choices = vec![choice(completion.clone())];
    assert_eq!(
        is_chunk_non_empty(
            &shaper,
            &completion,
            &response,
            &ChunkResponseObject::default()
        ),
        expected
    );
}

#[test]
fn non_empty_counts_provider_fields_from_the_response_object() {
    let shaper = shaper("gpt-4", Some("openai"));
    let response = chunk("c", 1);
    let response_obj = ChunkResponseObject {
        original_chunk: None,
        provider_specific_fields: Some(json_object(r#"{"a": 1}"#)),
    };
    assert!(is_chunk_non_empty(
        &shaper,
        &empty_delta(),
        &response,
        &response_obj
    ));
}

#[test]
fn non_empty_reads_the_remote_role_before_the_first_chunk_only() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut role = empty_delta();
    role.role = Some("assistant".to_owned());
    let mut response = chunk("c", 1);
    response.choices = vec![choice(role)];
    let response_obj = ChunkResponseObject::default();
    assert!(is_chunk_non_empty(
        &shaper,
        &empty_delta(),
        &response,
        &response_obj
    ));
    shaper.progress.sent_first_chunk = true;
    assert!(!is_chunk_non_empty(
        &shaper,
        &empty_delta(),
        &response,
        &response_obj
    ));
}

#[test]
fn non_empty_is_false_without_choices() {
    let shaper = shaper("gpt-4", Some("openai"));
    let mut response = chunk("c", 1);
    response.choices.clear();
    assert!(!is_chunk_non_empty(
        &shaper,
        &content_delta("x"),
        &response,
        &ChunkResponseObject::default()
    ));
}

#[rstest]
#[case::content(r#"{"content": "x"}"#, false)]
#[case::empty_content(r#"{"content": ""}"#, true)]
#[case::nothing("{}", true)]
#[case::tool_calls(r#"{"tool_calls": []}"#, false)]
#[case::function_call(r#"{"function_call": {"name": "f", "arguments": "{}"}}"#, false)]
fn non_empty_delta_emptiness_follows_litellm(#[case] source: &str, #[case] expected: bool) {
    let delta: StreamDelta = parse(source);
    assert_eq!(is_delta_empty(&delta), expected);
}

#[test]
fn processed_first_chunk_adds_the_assistant_role() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut completion = content_delta("hello");
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut completion,
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the chunk is sent");
    assert_eq!(sent.choices[0].delta.content.as_deref(), Some("hello"));
    assert_eq!(sent.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(sent.id, "chatcmpl-00000000-0000-4000-8000-000000000000");
    assert!(shaper.progress.sent_first_chunk);
}

#[test]
fn processed_later_chunks_rebuild_the_delta_without_a_role() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.progress.sent_first_chunk = true;
    let mut completion = content_delta("b");
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut completion,
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the chunk is sent");
    assert_eq!(sent.choices[0].delta.role, None);
    assert_eq!(sent.choices[0].delta.content.as_deref(), Some("b"));
}

#[test]
fn processed_index_and_provider_fields_reach_the_delta() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut completion = content_delta("hi");
    completion.extra.insert("index".to_owned(), number("4"));
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let response_obj = ChunkResponseObject {
        original_chunk: None,
        provider_specific_fields: Some(json_object(r#"{"a": 1}"#)),
    };
    let sent =
        return_processed_chunk_logic(&mut shaper, &mut completion, response, &response_obj, 5)
            .expect("no repetition here")
            .expect("the chunk is sent");
    assert_eq!(sent.choices[0].index, 4);
    assert_eq!(sent.choices[0].delta.extra.get("index"), Some(&number("4")));
    assert!(sent.choices[0].delta.provider_specific_fields.is_some());
}

#[test]
fn processed_original_chunk_replaces_choices_and_keeps_upstream_extras() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut completion = content_delta("hello");
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let mut original = chunk_with_content("orig", "orig-content");
    original.extra.insert(
        "service_tier".to_owned(),
        JsonValue::String("scale".to_owned()),
    );
    original.extra.insert(
        "citations".to_owned(),
        JsonValue::Array(vec![JsonValue::String("c1".to_owned())]),
    );
    original.system_fingerprint = Some("fp-orig".to_owned());
    original.choices[0].finish_reason = Some("length".to_owned());
    let response_obj = ChunkResponseObject {
        original_chunk: Some(original),
        provider_specific_fields: None,
    };
    let sent =
        return_processed_chunk_logic(&mut shaper, &mut completion, response, &response_obj, 5)
            .expect("no repetition here")
            .expect("the chunk is sent");
    assert_eq!(
        sent.choices[0].delta.content.as_deref(),
        Some("orig-content")
    );
    assert_eq!(sent.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(sent.choices[0].finish_reason, None);
    assert_eq!(sent.system_fingerprint.as_deref(), Some("fp-orig"));
    assert_eq!(
        sent.extra.get("service_tier"),
        Some(&JsonValue::String("scale".to_owned()))
    );
    assert!(sent.extra.contains_key("citations"));
}

#[test]
fn processed_original_chunk_without_choices_sends_nothing() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut completion = content_delta("hello");
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let mut original = chunk("orig", 5);
    original.choices.clear();
    let response_obj = ChunkResponseObject {
        original_chunk: Some(original),
        provider_specific_fields: None,
    };
    assert!(
        return_processed_chunk_logic(&mut shaper, &mut completion, response, &response_obj, 5)
            .expect("no repetition here")
            .is_none()
    );
}

#[test]
fn processed_completion_tool_calls_reach_the_delta_with_the_role() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut completion: StreamDelta = parse(r#"{"tool_calls": [{"id": "t"}]}"#);
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut completion,
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the chunk is sent");
    assert_eq!(sent.choices[0].delta.role.as_deref(), Some("assistant"));
    assert!(has_special_delta_content(&sent));
}

#[test]
fn processed_audio_only_delta_goes_through_the_special_path() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut response =
        model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let mut delta = empty_delta();
    delta.audio = Some(JsonValue::Object(json_object(r#"{"data": "x"}"#)));
    response.choices = vec![choice(delta)];
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut empty_delta(),
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the chunk is sent");
    assert_eq!(
        sent.choices[0].delta.audio,
        Some(JsonValue::Object(json_object(r#"{"data": "x"}"#)))
    );
}

#[test]
fn processed_finish_branch_flushes_the_holding_chunk() {
    let mut shaper = shaper("sagemaker", Some("sagemaker"));
    shaper.received_finish_reason = Some("stop".to_owned());
    shaper.holding_chunk = "HELD".to_owned();
    let mut completion = empty_delta();
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut completion,
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the flushed chunk is sent");
    assert_eq!(sent.choices[0].delta.content.as_deref(), Some("HELD"));
    assert!(shaper.holding_chunk.is_empty());
    assert!(!shaper.progress.sent_last_chunk);
}

#[test]
fn processed_finish_branch_maps_the_reason_on_an_empty_delta() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.received_finish_reason = Some("max_tokens".to_owned());
    let mut completion = empty_delta();
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut completion,
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the last chunk is sent");
    assert_eq!(sent.choices[0].finish_reason.as_deref(), Some("length"));
    assert_eq!(sent.choices[0].delta, empty_delta());
    assert!(shaper.progress.sent_last_chunk);
}

#[test]
fn processed_finish_branch_returns_a_non_empty_delta_unchanged() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.received_finish_reason = Some("stop".to_owned());
    let mut response =
        model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let mut delta = empty_delta();
    delta.audio = Some(JsonValue::Object(json_object(r#"{"data": "x"}"#)));
    response.choices = vec![choice(delta)];
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut empty_delta(),
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the chunk is sent unchanged");
    assert_eq!(sent.choices[0].finish_reason, None);
    assert!(!shaper.progress.sent_last_chunk);
    assert!(sent.choices[0].delta.audio.is_some());
}

#[test]
fn processed_finish_branch_keeps_a_usage_chunk_after_the_last_one() {
    let mut shaper = shaper("openrouter", Some("openrouter"));
    shaper.received_finish_reason = Some("stop".to_owned());
    shaper.progress.sent_last_chunk = true;
    let mut response =
        model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    response.usage = Some(usage_with(1, 2));
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut empty_delta(),
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the usage chunk is sent");
    assert!(sent.usage.is_some());
}

#[test]
fn processed_finish_branch_keeps_a_bedrock_trace_after_the_last_one() {
    let mut shaper = shaper("bedrock", Some("bedrock"));
    shaper.received_finish_reason = Some("stop".to_owned());
    shaper.progress.sent_last_chunk = true;
    let mut response =
        model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    response.extra.insert(
        "trace".to_owned(),
        JsonValue::Object(json_object(r#"{"guardrail": 1}"#)),
    );
    let sent = return_processed_chunk_logic(
        &mut shaper,
        &mut empty_delta(),
        response,
        &ChunkResponseObject::default(),
        5,
    )
    .expect("no repetition here")
    .expect("the trace chunk is sent");
    assert!(sent.extra.contains_key("trace"));
}

#[test]
fn processed_finish_branch_drops_everything_else_after_the_last_one() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.received_finish_reason = Some("stop".to_owned());
    shaper.progress.sent_last_chunk = true;
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    assert!(
        return_processed_chunk_logic(
            &mut shaper,
            &mut empty_delta(),
            response,
            &ChunkResponseObject::default(),
            5,
        )
        .expect("no repetition here")
        .is_none()
    );
}

#[test]
fn processed_finish_branch_preserves_original_chunk_extras() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.received_finish_reason = Some("stop".to_owned());
    let mut completion = empty_delta();
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    let mut original = chunk("orig", 1);
    original
        .extra
        .insert("provider_extra".to_owned(), JsonValue::Bool(true));
    let response_obj = ChunkResponseObject {
        original_chunk: Some(original),
        provider_specific_fields: None,
    };
    let sent =
        return_processed_chunk_logic(&mut shaper, &mut completion, response, &response_obj, 5)
            .expect("no repetition here")
            .expect("the last chunk is sent");
    assert_eq!(
        sent.extra.get("provider_extra"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn processed_usage_chunk_without_a_finish_reason_is_recorded_and_not_sent() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut response =
        model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    response.usage = Some(usage_with(1, 2));
    assert!(
        return_processed_chunk_logic(
            &mut shaper,
            &mut empty_delta(),
            response,
            &ChunkResponseObject::default(),
            5,
        )
        .expect("no repetition here")
        .is_none()
    );
    assert_eq!(shaper.chunks.len(), 1);
    assert_eq!(shaper.chunks[0].choices.len(), 1);
}

#[test]
fn processed_empty_chunk_is_neither_sent_nor_recorded() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    assert!(
        return_processed_chunk_logic(
            &mut shaper,
            &mut empty_delta(),
            response,
            &ChunkResponseObject::default(),
            5,
        )
        .expect("no repetition here")
        .is_none()
    );
    assert!(shaper.chunks.is_empty());
}

#[test]
fn processed_stream_usage_follows_include_usage() {
    let off: StreamOptions = parse("{}");
    let on: StreamOptions = parse(r#"{"include_usage": true}"#);
    assert!(!check_send_stream_usage(None));
    assert!(!check_send_stream_usage(Some(&off)));
    assert!(check_send_stream_usage(Some(&on)));
}

#[rstest]
#[case::functions(r#"{"functions": [{"name": "f"}]}"#, true)]
#[case::empty_functions(r#"{"functions": []}"#, false)]
#[case::tools_only(r#"{"tools": [{"type": "function"}]}"#, false)]
#[case::absent("{}", false)]
fn processed_function_call_reads_the_functions_param(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(check_is_function_call(Some(&json_object(source))), expected);
}

#[test]
fn thinking_first_reasoning_opens_the_tag_before_the_content() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.merge_reasoning_in_choices = true;
    let mut response = chunk("c", 1);
    let mut delta = empty_delta();
    delta.reasoning_content = Some("thinking1".to_owned());
    response.choices = vec![choice(delta)];
    combine_thinking_in_choices(&mut shaper, &mut response);
    assert_eq!(
        response.choices[0].delta.content.as_deref(),
        Some("<think>thinking1")
    );
    assert_eq!(response.choices[0].delta.reasoning_content, None);
    assert!(shaper.thinking.sent_first_block);
}

#[test]
fn thinking_later_reasoning_replaces_the_content() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.merge_reasoning_in_choices = true;
    shaper.thinking.sent_first_block = true;
    let mut response = chunk("c", 1);
    let mut delta = content_delta("visible");
    delta.reasoning_content = Some("thinking2".to_owned());
    response.choices = vec![choice(delta)];
    combine_thinking_in_choices(&mut shaper, &mut response);
    assert_eq!(
        response.choices[0].delta.content.as_deref(),
        Some("thinking2")
    );
    assert_eq!(response.choices[0].delta.reasoning_content, None);
}

#[test]
fn thinking_first_visible_content_closes_the_tag_once() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.merge_reasoning_in_choices = true;
    shaper.thinking.sent_first_block = true;
    let mut response = chunk("c", 1);
    response.choices = vec![choice(content_delta("answer"))];
    combine_thinking_in_choices(&mut shaper, &mut response);
    assert_eq!(
        response.choices[0].delta.content.as_deref(),
        Some("</think>answer")
    );
    assert!(shaper.thinking.sent_last_block);

    let mut later = chunk("c", 1);
    later.choices = vec![choice(content_delta("more"))];
    combine_thinking_in_choices(&mut shaper, &mut later);
    assert_eq!(later.choices[0].delta.content.as_deref(), Some("more"));
}

#[test]
fn thinking_merge_disabled_keeps_the_reasoning_content() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut response = chunk("c", 1);
    let mut delta = empty_delta();
    delta.reasoning_content = Some("r".to_owned());
    response.choices = vec![choice(delta)];
    combine_thinking_in_choices(&mut shaper, &mut response);
    assert_eq!(
        response.choices[0].delta.reasoning_content.as_deref(),
        Some("r")
    );
    assert_eq!(response.choices[0].delta.content, None);
}

#[test]
fn thinking_empty_reasoning_closes_the_block_on_the_next_content() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.merge_reasoning_in_choices = true;
    shaper.thinking.sent_first_block = true;
    let mut response = chunk("c", 1);
    let mut delta = content_delta("answer");
    delta.reasoning_content = Some(String::new());
    response.choices = vec![choice(delta)];
    combine_thinking_in_choices(&mut shaper, &mut response);
    assert_eq!(
        response.choices[0].delta.content.as_deref(),
        Some("</think>answer")
    );
}

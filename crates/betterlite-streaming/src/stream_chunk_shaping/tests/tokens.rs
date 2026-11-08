#[test]
fn repetition_raises_at_the_limit_with_litellms_message() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 4).expect("below the limit so far");
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 4).expect("below the limit so far");
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 4).expect("below the limit so far");
    shaper.chunks.push(chunk_with_content("c", "abc"));
    let error = raise_on_model_repetition(&mut shaper, 4).expect_err("the model repeats itself");
    assert_eq!(
        error.to_string(),
        "The model is repeating the same chunk = abc."
    );
    match error {
        StreamShapeError::Repetition {
            model, provider, ..
        } => {
            assert!(model.is_empty());
            assert!(provider.is_empty());
        }
        StreamShapeError::Arguments { .. } => {
            panic!("the repetition error was expected");
        }
    }
}

#[test]
fn repetition_below_the_limit_passes() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 4).expect("below the limit");
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 4).expect("below the limit");
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 4).expect("below the limit");
    assert_eq!(shaper.repeated_messages_count, 3);
}

#[test]
fn repetition_short_content_restarts_the_count() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.repeated_messages_count = 7;
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 100).expect("the pair repeats");
    shaper.chunks.push(chunk_with_content("c", "ab"));
    raise_on_model_repetition(&mut shaper, 100).expect("two characters cannot repeat");
    assert_eq!(shaper.repeated_messages_count, 1);
}

#[test]
fn repetition_different_content_restarts_the_count() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 100).expect("one chunk cannot repeat");
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 100).expect("the pair repeats");
    shaper.chunks.push(chunk_with_content("c", "xyz"));
    raise_on_model_repetition(&mut shaper, 100).expect("the last two differ");
    assert_eq!(shaper.repeated_messages_count, 1);
}

#[test]
fn repetition_skips_chunks_without_choices() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut first = chunk_with_content("c", "abc");
    first.choices.clear();
    shaper.chunks.push(first);
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 2).expect("a chunk without choices is not compared");
    assert_eq!(shaper.repeated_messages_count, 1);
}

#[test]
fn repetition_with_one_chunk_never_raises() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.chunks.push(chunk_with_content("c", "abc"));
    raise_on_model_repetition(&mut shaper, 1).expect("a single chunk cannot repeat");
}

#[test]
fn repetition_without_content_restarts_the_count() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.repeated_messages_count = 9;
    shaper.chunks.push(chunk_with_content("c", "abc"));
    shaper.chunks.push(chunk("c", 1));
    raise_on_model_repetition(&mut shaper, 2).expect("no content is not a repeat");
    assert_eq!(shaper.repeated_messages_count, 1);
}

#[rstest]
#[case::partial_prefix("<", true)]
#[case::partial_suffix("|im_end", true)]
#[case::plain_text("hello", false)]
#[case::empty("", true)]
fn special_tokens_hold_a_partial_token_before_the_first_chunk(
    #[case] piece: &str,
    #[case] expected: bool,
) {
    let mut shaper = shaper("sagemaker", Some("sagemaker"));
    let (hold, text) = check_special_tokens(&mut shaper, piece, None);
    assert_eq!(hold, expected, "text was {text:?}");
}

#[test]
fn special_tokens_hold_the_rest_of_a_token_across_chunks() {
    let mut shaper = shaper("sagemaker", Some("sagemaker"));
    let (first_hold, first_text) = check_special_tokens(&mut shaper, "<", None);
    assert!(first_hold);
    assert_eq!(first_text, "<");
    assert_eq!(shaper.holding_chunk, "<");
    let (second_hold, second_text) = check_special_tokens(&mut shaper, "|im_end|>", None);
    assert!(second_hold);
    assert_eq!(second_text, "<|im_end|>");
    assert!(shaper.holding_chunk.is_empty());
}

#[test]
fn special_tokens_hold_a_complete_token_and_keep_the_text() {
    let mut shaper = shaper("sagemaker", Some("sagemaker"));
    let (hold, text) = check_special_tokens(&mut shaper, "<s>", None);
    assert!(hold);
    assert_eq!(text, "<s>");
    assert!(shaper.holding_chunk.is_empty());
}

#[test]
fn special_tokens_are_stripped_on_the_finish_reason_chunk() {
    let mut shaper = shaper("sagemaker", Some("sagemaker"));
    let (hold, text) = check_special_tokens(&mut shaper, "bye<|assistant|>", Some("stop"));
    assert!(!hold);
    assert_eq!(text, "bye");
}

#[test]
fn special_tokens_pass_through_after_the_first_chunk() {
    let mut shaper = shaper("sagemaker", Some("sagemaker"));
    shaper.progress.sent_first_chunk = true;
    let (hold, text) = check_special_tokens(&mut shaper, "<|im_end|>", None);
    assert!(!hold);
    assert_eq!(text, "<|im_end|>");
}

#[test]
fn special_tokens_pass_through_for_other_providers() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let (hold, text) = check_special_tokens(&mut shaper, "<|im_end|>", None);
    assert!(!hold);
    assert_eq!(text, "<|im_end|>");
    assert!(shaper.holding_chunk.is_empty());
}

#[test]
fn finish_reason_handler_prefers_received_then_intermittent_then_stop() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let defaulted = finish_reason_handler(&mut shaper);
    assert_eq!(defaulted.choices[0].finish_reason.as_deref(), Some("stop"));

    shaper.intermittent_finish_reason = Some("content_filter".to_owned());
    let intermittent = finish_reason_handler(&mut shaper);
    assert_eq!(
        intermittent.choices[0].finish_reason.as_deref(),
        Some("content_filter")
    );

    shaper.received_finish_reason = Some("length".to_owned());
    let received = finish_reason_handler(&mut shaper);
    assert_eq!(received.choices[0].finish_reason.as_deref(), Some("length"));
}

#[test]
fn finish_reason_handler_turns_stop_into_tool_calls_after_a_tool_call() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.tool_call = true;
    let response = finish_reason_handler(&mut shaper);
    assert_eq!(
        response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
}

#[test]
fn finish_reason_handler_keeps_other_reasons_with_a_tool_call() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.tool_call = true;
    shaper.received_finish_reason = Some("content_filter".to_owned());
    let response = finish_reason_handler(&mut shaper);
    assert_eq!(
        response.choices[0].finish_reason.as_deref(),
        Some("content_filter")
    );
}

#[derive(Clone, Default)]
struct CapturedEvents(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for CapturedEvents {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut fields = CapturedFields::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.extend(fields.0);
        }
    }
}

#[derive(Default)]
struct CapturedFields(Vec<String>);

impl Visit for CapturedFields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0.push(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push(value.to_owned());
        }
    }
}

#[test]
fn finish_reason_unknown_value_warns_and_falls_back_to_stop() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturedEvents(Arc::clone(&events)));
    let mapped = with_default(subscriber, || mapped_finish_reason("weird"));
    assert_eq!(mapped, "stop");
    let captured = events.lock().expect("the capture lock is free").clone();
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("Unmapped finish_reason 'weird', defaulting to 'stop'"));
}

#[rstest]
#[case::length("max_tokens", "length")]
#[case::anthropic_tool_use("tool_use", "tool_calls")]
#[case::content_filtered("content_filtered", "content_filter")]
#[case::cohere_complete("COMPLETE", "stop")]
#[case::passthrough("function_call", "function_call")]
#[case::generic_error("error", "stop")]
#[case::joined_length_values("max_tokens MAX_TOKENS", "stop")]
#[case::joined_stop_values("stop error", "stop")]
fn finish_reason_maps_the_table(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(mapped_finish_reason(raw), expected);
}

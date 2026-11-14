#[test]
fn transform_chunk_emits_only_the_new_text() {
    let mutated = text_map(&[(0, "Hello world")]);
    let mut emitted = text_map(&[(0, "Hello")]);
    let mut held = IndexMap::new();
    let reference = chunk("Hello");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": " world", "role": null, "tool_calls": null}, "finish_reason": null}]}"#
        )
    );
    assert_eq!(emitted.get(&0).map(String::as_str), Some("Hello world"));
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_keeps_the_holdback_before_the_final_round() {
    let mutated = text_map(&[(0, "Hello world")]);
    let mut emitted = IndexMap::new();
    let holdback = index_map(&[(0, 5)]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "Hello ", "role": "assistant", "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
    assert_eq!(held.get(&0), Some(&5));
    assert_eq!(emitted.get(&0).map(String::as_str), Some("Hello "));
}

#[test]
fn transform_chunk_forces_the_holdback_to_zero_on_the_final_round() {
    let mutated = text_map(&[(0, "Hello world")]);
    let mut emitted = IndexMap::new();
    let holdback = index_map(&[(0, 5)]);
    let mut held = index_map(&[(0, 5)]);
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &reason_map(&[(0, Some("stop"))]),
        held_chars_per_choice: &mut held,
        is_final: true,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "Hello world", "role": "assistant", "tool_calls": null}, "finish_reason": "stop"}]}"#
        )
    );
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_missing_holdback_counts_as_zero() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_negative_holdback_counts_as_zero() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &index_map(&[(0, -3)]),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_counts_characters_not_bytes() {
    let mutated = text_map(&[(0, "h\u{e9}llo w\u{f6}rld")]);
    let mut emitted = text_map(&[(0, "h\u{e9}llo")]);
    let holdback = index_map(&[(0, 5)]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": " ", "role": null, "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
    assert_eq!(held.get(&0), Some(&5));
    assert_eq!(emitted.get(&0).map(String::as_str), Some("h\u{e9}llo "));
}

#[test]
fn transform_chunk_holds_everything_without_a_chunk() {
    let mutated = text_map(&[(0, "Hi")]);
    let mut emitted = IndexMap::new();
    let holdback = index_map(&[(0, 5)]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap();
    assert!(output.is_none());
    assert_eq!(held.get(&0), Some(&2));
    assert!(emitted.is_empty());
}

#[test]
fn transform_chunk_underflow_returns_the_400() {
    let mutated = text_map(&[(2, "Hx")]);
    let mut emitted = text_map(&[(2, "Hello")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let error = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap_err();
    assert_eq!(error.status_code, 400);
    let expected = "Guardrail streaming transform for choice 2 is not a forward extension of the 5 chars already streamed to the client (it is shorter than, or rewrites, the emitted prefix); emitted bytes cannot be retracted. Withhold recent output via stream_holdback_chars before rewriting it.";
    assert_eq!(
        error.detail,
        HttpErrorDetail::Object(BTreeMap::from([
            ("error".to_owned(), text("stream_transform_underflow")),
            ("message".to_owned(), text(expected)),
        ]))
    );
    assert_eq!(emitted.len(), 1);
}

#[test]
fn transform_chunk_underflow_fails_the_whole_round() {
    let mutated = text_map(&[(0, "ab"), (1, "xy")]);
    let mut emitted = text_map(&[(0, "a"), (1, "zz")]);
    let mut held = index_map(&[(0, 0), (1, 0)]);
    let reference = chunk("");
    let error = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &index_map(&[(0, 1), (1, 1)]),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap_err();
    assert_eq!(error.status_code, 400);
    assert_eq!(emitted.len(), 2);
    assert_eq!(emitted.get(&0).map(String::as_str), Some("a"));
    assert_eq!(emitted.get(&1).map(String::as_str), Some("zz"));
}

#[test]
fn transform_chunk_terminator_carries_the_finish_reasons() {
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some("stop")), (1, None)]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "", "role": null, "tool_calls": null}, "finish_reason": "stop"}]}"#
        )
    );
}

#[test]
fn transform_chunk_terminator_needs_the_final_round() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some("stop"))]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: false,
    })
    .unwrap();
    assert!(output.is_none());
}

#[test]
fn transform_chunk_terminator_without_reasons_is_none() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, None), (1, None)]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap();
    assert!(output.is_none());
}

#[test]
fn transform_chunk_maps_an_anthropic_finish_reason() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some("end_turn"))]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "", "role": null, "tool_calls": null}, "finish_reason": "stop"}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn transform_chunk_finish_reason_only_counts_on_the_final_round() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = text_map(&[(0, "a")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some("stop"))]),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "b", "role": null, "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn transform_chunk_role_is_assistant_only_on_the_first_delta() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    let first = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "ab", "role": "assistant", "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&first), expected_choices(expected));
    let next = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &text_map(&[(0, "abc")]),
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let next_expected = parsed(
        r#"[{"index": 0, "delta": {"content": "c", "role": null, "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&next), expected_choices(next_expected));
}

#[test]
fn transform_chunk_role_is_assistant_when_the_emitted_text_is_empty() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = text_map(&[(0, "")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "ab", "role": "assistant", "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn transform_chunk_skips_a_choice_with_nothing_to_say() {
    let mutated = text_map(&[(0, "ab"), (1, "cd")]);
    let mut emitted = text_map(&[(0, "ab")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 1, "delta": {"content": "cd", "role": "assistant", "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
    assert_eq!(emitted.get(&0).map(String::as_str), Some("ab"));
    assert_eq!(emitted.get(&1).map(String::as_str), Some("cd"));
}

#[test]
fn transform_chunk_returns_none_when_no_choice_is_left() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = text_map(&[(0, "ab")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap();
    assert!(output.is_none());
    assert_eq!(emitted.get(&0).map(String::as_str), Some("ab"));
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_inserts_an_empty_emitted_text_for_a_skipped_choice() {
    let mutated = text_map(&[(0, ""), (1, "cd")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(emitted.get(&0).map(String::as_str), Some(""));
    assert_eq!(emitted.get(&1).map(String::as_str), Some("cd"));
}

#[test]
fn transform_chunk_keeps_the_mutated_choice_order() {
    let mutated = text_map(&[(2, "bb"), (0, "aa")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(choice_indexes(chunk_choices(&output)), vec![2, 0]);
}

#[test]
fn transform_chunk_missing_reference_fields_are_null() {
    let reference = GuardrailValue::Object(BTreeMap::new());
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &text_map(&[(0, "a")]),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": null, "created": null, "model": null, "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "a", "role": "assistant", "tool_calls": null}, "finish_reason": null}]}"#
        )
    );
}

#[test]
fn transform_chunk_empty_finish_reason_is_no_reason_in_the_chunk() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some(""))]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "", "role": null, "tool_calls": null}, "finish_reason": null}]}"#
        )
    );
}

#[test]
fn transform_chunk_empty_reason_keeps_a_choice_with_no_delta() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &text_map(&[(0, "")]),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some(""))]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "", "role": "assistant", "tool_calls": null}, "finish_reason": null}]}"#
        )
    );
}

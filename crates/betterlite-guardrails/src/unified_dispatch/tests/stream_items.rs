#[test]
fn stream_item_choices_are_read_from_the_array() {
    let item = chunk("hi");
    assert_eq!(chunk_choices(&item).len(), 1);
}

#[rstest]
#[case::text(text("data: {}"))]
#[case::null(GuardrailValue::Null)]
#[case::array(GuardrailValue::Array(Vec::new()))]
#[case::no_choices(parsed(r#"{"id": "1"}"#))]
#[case::choices_not_an_array(parsed(r#"{"choices": "none"}"#))]
fn stream_item_choices_are_empty(#[case] item: GuardrailValue) {
    assert!(chunk_choices(&item).is_empty());
}

#[rstest]
#[case::text(parsed(r#"{"choices": [{"delta": {"content": "hi"}}]}"#), true)]
#[case::empty(parsed(r#"{"choices": [{"delta": {"content": ""}}]}"#), false)]
#[case::null(parsed(r#"{"choices": [{"delta": {"content": null}}]}"#), false)]
#[case::missing(parsed(r#"{"choices": [{"delta": {}}]}"#), false)]
#[case::no_delta(parsed(r#"{"choices": [{"index": 0}]}"#), false)]
#[case::other_choice(
    parsed(r#"{"choices": [{"delta": {"content": ""}}, {"delta": {"content": "x"}}]}"#),
    true
)]
fn stream_item_carries_text(#[case] item: GuardrailValue, #[case] expected: bool) {
    assert_eq!(chunk_carries_text(&item), expected);
}

#[rstest]
#[case::calls(
    parsed(r#"{"choices": [{"delta": {"tool_calls": [{"id": "c"}]}}]}"#),
    true
)]
#[case::empty(parsed(r#"{"choices": [{"delta": {"tool_calls": []}}]}"#), false)]
#[case::null(parsed(r#"{"choices": [{"delta": {"tool_calls": null}}]}"#), false)]
#[case::missing(parsed(r#"{"choices": [{"delta": {"content": "hi"}}]}"#), false)]
#[case::no_choices(text("frame"), false)]
fn stream_item_has_tool_calls(#[case] item: GuardrailValue, #[case] expected: bool) {
    assert_eq!(chunk_has_tool_calls(&item), expected);
}

#[rstest]
#[case::reason(parsed(r#"{"choices": [{"finish_reason": "stop"}]}"#), true)]
#[case::null(parsed(r#"{"choices": [{"finish_reason": null}]}"#), false)]
#[case::empty(parsed(r#"{"choices": [{"finish_reason": ""}]}"#), false)]
#[case::second_choice(
    parsed(r#"{"choices": [{"finish_reason": null}, {"finish_reason": "length"}]}"#),
    true
)]
fn stream_item_has_finish_reason(#[case] item: GuardrailValue, #[case] expected: bool) {
    assert_eq!(chunk_has_finish_reason(&item), expected);
}

#[rstest]
#[case::no_choices(parsed(r#"{"id": "1"}"#), true)]
#[case::frame(text("frame"), true)]
#[case::usage_only(
    parsed(r#"{"choices": [{"delta": {"content": ""}}], "usage": {"total_tokens": 3}}"#),
    true
)]
#[case::usage_with_text(
    parsed(r#"{"choices": [{"delta": {"content": "hi"}}], "usage": {"total_tokens": 3}}"#),
    false
)]
#[case::usage_with_finish(
    parsed(
        r#"{"choices": [{"delta": {"content": ""}, "finish_reason": "stop"}], "usage": {"total_tokens": 3}}"#
    ),
    false
)]
#[case::no_usage(parsed(r#"{"choices": [{"delta": {"content": ""}}]}"#), false)]
#[case::null_usage(
    parsed(r#"{"choices": [{"delta": {"content": ""}}], "usage": null}"#),
    false
)]
fn stream_item_trailing_metadata(#[case] item: GuardrailValue, #[case] expected: bool) {
    assert_eq!(is_trailing_metadata_chunk(&item), expected);
}

#[test]
fn stream_item_trailing_metadata_chunks_keep_the_order() {
    let items = [
        parsed(r#"{"id": "a"}"#),
        chunk("hi"),
        parsed(r#"{"choices": [], "usage": {"total_tokens": 3}}"#),
        parsed(r#"{"id": "b"}"#),
    ];
    let selected: Vec<&GuardrailValue> = trailing_metadata_chunks(&items);
    assert_eq!(selected.len(), 3);
    assert_eq!(selected.first().copied(), items.first());
    assert_eq!(selected.get(2).copied(), items.get(3));
}

#[test]
fn stream_item_records_finish_reasons_by_choice_index() {
    let item = parsed(
        r#"{"choices": [{"index": 0, "finish_reason": "stop"}, {"index": 1, "finish_reason": null}, {"index": 2, "finish_reason": "end_turn"}]}"#,
    );
    let mut reasons = IndexMap::new();
    record_finish_reasons(&item, &mut reasons);
    assert_eq!(reasons.get(&0), Some(&Some("stop".to_owned())));
    assert_eq!(reasons.get(&1), None);
    assert_eq!(reasons.get(&2), Some(&Some("stop".to_owned())));
}

#[test]
fn stream_item_choice_index_reads_an_unsigned_index() {
    let item = parsed(r#"{"choices": [{"index": 18446744073709551615, "finish_reason": "stop"}]}"#);
    let mut reasons = IndexMap::new();
    record_finish_reasons(&item, &mut reasons);
    assert_eq!(reasons.get(&i64::MAX), Some(&Some("stop".to_owned())));
}

#[test]
fn stream_item_records_a_null_index_as_choice_zero() {
    let item = parsed(r#"{"choices": [{"index": null, "finish_reason": "length"}]}"#);
    let mut reasons = IndexMap::new();
    record_finish_reasons(&item, &mut reasons);
    assert_eq!(reasons.get(&0), Some(&Some("length".to_owned())));
}

#[test]
fn stream_item_records_skip_an_empty_reason() {
    let item = parsed(r#"{"choices": [{"index": 0, "finish_reason": ""}]}"#);
    let mut reasons = IndexMap::new();
    record_finish_reasons(&item, &mut reasons);
    assert!(reasons.is_empty());
}

#[test]
fn stream_item_tool_call_passthrough_strips_the_text() {
    let item = parsed(
        r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "secret", "role": "assistant", "tool_calls": [{"id": "call_1", "type": "function"}]}, "finish_reason": null}]}"#,
    );
    let output = tool_call_passthrough_chunk(&item, None, &BTreeSet::new());
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": null, "role": "assistant", "tool_calls": [{"id": "call_1", "type": "function"}]}, "finish_reason": null}]}"#
        )
    );
}

#[test]
fn stream_item_tool_call_passthrough_defers_a_text_choice_reason() {
    let item = parsed(
        r#"{"choices": [{"index": 3, "delta": {"content": "secret", "tool_calls": [{"id": "c"}]}, "finish_reason": "end_turn"}]}"#,
    );
    let mut reasons = IndexMap::new();
    let output = tool_call_passthrough_chunk(&item, Some(&mut reasons), &BTreeSet::new());
    assert_eq!(reasons.get(&3), Some(&Some("stop".to_owned())));
    let expected = parsed(
        r#"[{"index": 3, "delta": {"content": null, "role": null, "tool_calls": [{"id": "c"}]}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_tool_call_passthrough_defers_a_held_choice_reason() {
    let item = parsed(
        r#"{"choices": [{"index": 1, "delta": {"tool_calls": [{"id": "c"}]}, "finish_reason": "tool_calls"}]}"#,
    );
    let mut reasons = IndexMap::new();
    let held = BTreeSet::from([1]);
    let output = tool_call_passthrough_chunk(&item, Some(&mut reasons), &held);
    assert_eq!(reasons.get(&1), Some(&Some("tool_calls".to_owned())));
    let expected = parsed(
        r#"[{"index": 1, "delta": {"content": null, "role": null, "tool_calls": [{"id": "c"}]}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_tool_call_passthrough_keeps_other_reasons() {
    let item = parsed(
        r#"{"choices": [{"index": 0, "delta": {"tool_calls": [{"id": "c"}]}, "finish_reason": "tool_calls"}]}"#,
    );
    let mut reasons = IndexMap::new();
    let output = tool_call_passthrough_chunk(&item, Some(&mut reasons), &BTreeSet::new());
    assert!(reasons.is_empty());
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": null, "role": null, "tool_calls": [{"id": "c"}]}, "finish_reason": "tool_calls"}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_tool_call_passthrough_without_a_map_keeps_the_reason() {
    let item = parsed(
        r#"{"choices": [{"index": 0, "delta": {"content": "secret"}, "finish_reason": "stop"}]}"#,
    );
    let output = tool_call_passthrough_chunk(&item, None, &BTreeSet::new());
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": null, "role": null, "tool_calls": null}, "finish_reason": "stop"}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_tool_call_passthrough_maps_the_reason() {
    let item = parsed(r#"{"choices": [{"index": 0, "delta": {}, "finish_reason": "end_turn"}]}"#);
    let output = tool_call_passthrough_chunk(&item, None, &BTreeSet::new());
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": null, "role": null, "tool_calls": null}, "finish_reason": "stop"}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_held_choices_keep_only_positive_counts() {
    let held = index_map(&[(0, 2), (1, 0), (2, -1), (3, 5)]);
    assert_eq!(held_choices(&held), BTreeSet::from([0, 3]));
}

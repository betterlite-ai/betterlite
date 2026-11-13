#[rstest]
#[case::known(r#"{"text": "a", "is_finished": false, "finish_reason": ""}"#, true)]
#[case::missing_required(r#"{"text": "a"}"#, true)]
#[case::unknown_key(r#"{"text": "a", "bogus": 1}"#, false)]
#[case::joined_keys(r#"{"text tool_use": 1}"#, false)]
fn generic_known_fields_lists_the_typed_dict_keys(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(
        generic_chunk_has_known_fields(&json_object(source)),
        expected
    );
}

#[test]
fn generic_conversion_builds_the_chunk() {
    let generic: GenericChunk = parse(
        r#"{"text": "hi", "is_finished": true, "finish_reason": "length", "index": 2, "tool_use": {"id": "t"}, "usage": {"prompt_tokens": 1}}"#,
    );
    let response = convert_generic_chunk(&generic, fixed_instant, || Uuid::from_u128(7));
    assert_eq!(response.id, Uuid::from_u128(7).to_string());
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.model, "");
    assert_eq!(response.choices[0].index, 2);
    assert_eq!(response.choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(
        response.choices[0].delta.tool_calls,
        Some(JsonValue::Object(json_object(r#"{"id": "t"}"#)))
    );
    assert_eq!(response.choices[0].finish_reason, None);
    assert_eq!(
        response.extra.get("finish_reason"),
        Some(&JsonValue::String("length".to_owned()))
    );
    assert_eq!(response.usage.map(|usage| usage.prompt_tokens), Some(1));
}

#[test]
fn generic_conversion_without_a_finish_reason_writes_null() {
    let generic: GenericChunk =
        parse(r#"{"text": "hi", "is_finished": false, "finish_reason": ""}"#);
    let response = convert_generic_chunk(&generic, fixed_instant, || Uuid::from_u128(7));
    assert_eq!(response.choices[0].index, 0);
    assert_eq!(response.extra.get("finish_reason"), Some(&JsonValue::Null));
    assert_eq!(response.usage, None);
}

#[test]
fn sse_data_lines_strip_only_the_protocol_prefix() {
    assert_eq!(strip_sse_data_from_chunk(Some("data: {}")), Some("{}"));
    assert_eq!(strip_sse_data_from_chunk(Some("data:{}")), Some("{}"));
    assert_eq!(strip_sse_data_from_chunk(Some("data:")), Some(""));
    assert_eq!(strip_sse_data_from_chunk(Some("data")), Some("data"));
    assert_eq!(strip_sse_data_from_chunk(Some("")), Some(""));
    assert_eq!(
        strip_sse_data_from_chunk(Some(" data: {}")),
        Some(" data: {}")
    );
    assert_eq!(strip_sse_data_from_chunk(Some("data:  {}")), Some(" {}"));
    assert_eq!(strip_sse_data_from_chunk(None), None);
}

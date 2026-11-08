#[test]
fn creator_keeps_one_id_created_and_fingerprint_across_the_stream() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let first = creator(
        &mut shaper,
        r#"{"id": "chatcmpl-a", "created": 100, "model": "provider-model"}"#,
        None,
    );
    assert_eq!(first.id, "chatcmpl-a");
    assert_eq!(first.created, 100);
    assert_eq!(first.model, "gpt-4");
    assert_eq!(first.object, "chat.completion.chunk");
    assert_eq!(shaper.response_id.as_deref(), Some("chatcmpl-a"));
    assert_eq!(shaper.created, Some(100));

    shaper.system_fingerprint = Some("fp-1".to_owned());
    let second = creator(
        &mut shaper,
        r#"{"id": "chatcmpl-b", "created": 999, "system_fingerprint": "fp-chunk", "object": "other"}"#,
        None,
    );
    assert_eq!(second.id, "chatcmpl-a");
    assert_eq!(second.created, 100);
    assert_eq!(second.system_fingerprint.as_deref(), Some("fp-1"));
    assert_eq!(second.object, "chat.completion.chunk");
}

#[test]
fn creator_generates_the_identity_when_the_chunk_has_none() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let response = model_response_creator(&mut shaper, None, None).expect("no chunk cannot fail");
    assert!(response.id.starts_with("chatcmpl-"));
    assert_eq!(shaper.response_id.as_deref(), Some(response.id.as_str()));
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].index, 0);
    assert_eq!(response.choices[0].delta, empty_delta());
}

#[test]
fn creator_merges_hidden_params_with_the_base_winning() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let response = creator(
        &mut shaper,
        r#"{"id": "chatcmpl-a", "created": 100}"#,
        Some(
            r#"{"provider_response_model": "m2", "model_id": "caller-id", "custom_llm_provider": "caller"}"#,
        ),
    );
    assert_eq!(
        response.hidden_params.get("model_id"),
        Some(&JsonValue::String("m1".to_owned()))
    );
    assert_eq!(
        response.hidden_params.get("custom_llm_provider"),
        Some(&JsonValue::String("openai".to_owned()))
    );
    assert_eq!(
        response.hidden_params.get("provider_response_model"),
        Some(&JsonValue::String("m2".to_owned()))
    );
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&JsonValue::Null)
    );
    assert!(matches!(
        response.hidden_params.get("created_at"),
        Some(JsonValue::Number(_))
    ));
}

#[test]
fn creator_keeps_usage_choices_and_provider_fields_from_the_arguments() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let response = creator(
        &mut shaper,
        r#"{"id": "chatcmpl-a", "created": 1, "index": 3, "usage": {"prompt_tokens": 3, "completion_tokens": 4},
            "choices": [{"index": 3, "delta": {"content": "z"}}], "provider_specific_fields": {"a": 1}}"#,
        None,
    );
    assert_eq!(response.choices[0].index, 3);
    assert_eq!(response.choices[0].delta.content.as_deref(), Some("z"));
    assert_eq!(response.usage.expect("usage is kept").prompt_tokens, 3);
    assert!(response.provider_specific_fields.is_some());
    assert_eq!(response.extra.get("index"), Some(&number("3")));
}

#[test]
fn creator_without_a_provider_writes_a_null_provider() {
    let mut shaper = shaper("gpt-4", None);
    let response = creator(&mut shaper, r#"{"id": "chatcmpl-a", "created": 1}"#, None);
    assert_eq!(
        response.hidden_params.get("custom_llm_provider"),
        Some(&JsonValue::Null)
    );
}

#[rstest]
#[case::present(r#"{"provider_specific_fields": {"a": 1}}"#, true)]
#[case::null(r#"{"provider_specific_fields": null}"#, true)]
#[case::absent("{}", true)]
#[case::wrong_type(r#"{"provider_specific_fields": 5}"#, false)]
fn hidden_parsed_provider_hidden_params_validates_the_fields(
    #[case] source: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        parsed_provider_hidden_params(&json_object(source)).is_some(),
        expected
    );
}

#[test]
fn hidden_provider_hidden_params_carries_the_model_and_fields() {
    let mut provider_chunk = chunk("c", 1);
    provider_chunk.hidden_params = json_object(r#"{"provider_specific_fields": {"a": 1}}"#);
    let params = provider_hidden_params(&provider_chunk, Some("gpt-x")).expect("both values land");
    assert_eq!(
        params.get("provider_response_model"),
        Some(&JsonValue::String("gpt-x".to_owned()))
    );
    assert!(params.contains_key("provider_specific_fields"));
}

#[test]
fn hidden_provider_hidden_params_without_values_is_none() {
    let provider_chunk = chunk("c", 1);
    assert_eq!(provider_hidden_params(&provider_chunk, None), None);
}

#[test]
fn hidden_provider_hidden_params_skips_empty_fields() {
    let mut provider_chunk = chunk("c", 1);
    provider_chunk.hidden_params = json_object(r#"{"provider_specific_fields": {}}"#);
    assert_eq!(provider_hidden_params(&provider_chunk, None), None);
}

#[rstest]
#[case::model("gpt-x", Some("gpt-x"))]
#[case::blank("", None)]
fn hidden_provider_response_model_reads_the_chunk_model(
    #[case] model: &str,
    #[case] expected: Option<&str>,
) {
    let mut provider_chunk = chunk("c", 1);
    provider_chunk.model = model.to_owned();
    assert_eq!(provider_response_model(&provider_chunk), expected);
}

#[test]
fn hidden_stamp_served_service_tier_copies_the_complete_response_tier() {
    let mut response = chunk("c", 1);
    let mut complete = chunk("d", 1);
    complete.extra.insert(
        "service_tier".to_owned(),
        JsonValue::String("scale".to_owned()),
    );
    stamp_served_service_tier(&mut response, &complete);
    assert_eq!(
        response.extra.get("service_tier"),
        Some(&JsonValue::String("scale".to_owned()))
    );
}

#[test]
fn hidden_stamp_served_service_tier_ignores_a_blank_tier() {
    let mut response = chunk("c", 1);
    let mut complete = chunk("d", 1);
    complete
        .extra
        .insert("service_tier".to_owned(), JsonValue::String(String::new()));
    stamp_served_service_tier(&mut response, &complete);
    assert!(!response.extra.contains_key("service_tier"));
}

#[test]
fn hidden_set_model_id_adopts_the_first_id_and_records_it() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut response = chunk("c", 1);
    set_model_id(&mut shaper, " chatcmpl-provider ", &mut response);
    assert_eq!(shaper.response_id.as_deref(), Some(" chatcmpl-provider "));
    assert_eq!(response.id, " chatcmpl-provider ");
    assert_eq!(
        response.hidden_params.get("received_model_id"),
        Some(&JsonValue::String(" chatcmpl-provider ".to_owned()))
    );
}

#[test]
fn hidden_set_model_id_ignores_a_blank_id() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    shaper.response_id = Some("chatcmpl-fixed".to_owned());
    let mut response = chunk("c", 1);
    set_model_id(&mut shaper, "   ", &mut response);
    assert_eq!(shaper.response_id.as_deref(), Some("chatcmpl-fixed"));
    assert_eq!(response.id, "chatcmpl-fixed");
    assert!(!response.hidden_params.contains_key("received_model_id"));
}

#[test]
fn hidden_copy_provider_fields_lands_on_both_levels() {
    let mut original = chunk("c", 1);
    original.provider_specific_fields = Some(json_object(r#"{"a": 1}"#));
    let mut response = chunk("d", 1);
    copy_provider_specific_fields(&original, &mut response);
    assert!(response.provider_specific_fields.is_some());
    assert_eq!(response.extra.get("a"), Some(&number("1")));
}

#[test]
fn hidden_special_attributes_copy_only_audio_and_images() {
    let mut source = empty_delta();
    source.audio = Some(JsonValue::Object(json_object(r#"{"data": "x"}"#)));
    let mut target = empty_delta();
    assert!(has_any_special_attributes(&source));
    assert!(has_special_delta_attribute(
        &source,
        SpecialDeltaAttribute::Audio
    ));
    assert!(!has_special_delta_attribute(
        &source,
        SpecialDeltaAttribute::Images
    ));
    copy_delta_attribute(&source, &mut target, SpecialDeltaAttribute::Audio);
    assert_eq!(target.audio, source.audio);
    assert_eq!(target.images, None);

    let mut response = chunk("c", 1);
    apply_special_delta_attributes(&source, &mut response);
    assert!(response.choices[0].delta.audio.is_some());
}

#[test]
fn usage_total_takes_the_latest_counters_and_ignores_an_earlier_cost() {
    let mut first = usage_chunk(usage_with(10, 2));
    first.usage.as_mut().expect("usage is set").cost = Some(number("1"));
    let second = usage_chunk(usage_with(10, 5));
    let total = calculate_total_usage(&[first, second]);
    assert_eq!(total.prompt_tokens, 10);
    assert_eq!(total.completion_tokens, 5);
    assert_eq!(total.total_tokens, 15);
    assert_eq!(total.cost, None);
}

#[test]
fn usage_total_keeps_the_latest_cost_and_details() {
    let mut only = usage_chunk(usage_with(4, 6));
    let usage = only.usage.as_mut().expect("usage is set");
    usage.cost = Some(number("1"));
    usage.prompt_tokens_details = Some(details(r#"{"cached_tokens": 3}"#));
    let total = calculate_total_usage(&[only]);
    assert_eq!(total.total_tokens, 10);
    assert_eq!(total.cost, Some(number("1")));
    assert_eq!(
        total
            .prompt_tokens_details
            .as_ref()
            .and_then(|counted| counted.cached_tokens),
        Some(3)
    );
}

#[test]
fn usage_total_without_usage_is_zero() {
    let total = calculate_total_usage(&[chunk("c", 1)]);
    assert_eq!(total.prompt_tokens, 0);
    assert_eq!(total.completion_tokens, 0);
    assert_eq!(total.total_tokens, 0);
    assert_eq!(total.cost, None);
    assert_eq!(total.prompt_tokens_details, None);
}

#[test]
fn usage_total_carries_cache_creation_details_from_an_earlier_chunk() {
    let mut first = usage_chunk(usage_with(1, 1));
    first
        .usage
        .as_mut()
        .expect("usage is set")
        .prompt_tokens_details = Some(details(
        r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 7}}"#,
    ));
    let mut second = usage_chunk(usage_with(2, 2));
    second
        .usage
        .as_mut()
        .expect("usage is set")
        .prompt_tokens_details = Some(details(r#"{"cached_tokens": 4}"#));
    let total = calculate_total_usage(&[first, second]);
    let counted = total
        .prompt_tokens_details
        .expect("the last details survive");
    assert_eq!(counted.cached_tokens, Some(4));
    assert_eq!(
        counted.cache_creation_token_details,
        Some(parse::<JsonValue>(r#"{"ephemeral_5m_input_tokens": 7}"#))
    );
}

#[test]
fn usage_backfill_fills_the_cache_fields_from_the_prompt_details() {
    let mut usage = usage_with(1, 1);
    usage.prompt_tokens_details = Some(details(
        r#"{"cached_tokens": 3, "cache_creation_tokens": 4}"#,
    ));
    backfill_cache_usage(&mut usage);
    assert_eq!(
        usage.extra.get("cache_read_input_tokens"),
        Some(&number("3"))
    );
    assert_eq!(
        usage.extra.get("cache_creation_input_tokens"),
        Some(&number("4"))
    );
}

#[test]
fn usage_backfill_keeps_an_explicit_cache_field() {
    let mut usage = usage_with(1, 1);
    usage
        .extra
        .insert("cache_read_input_tokens".to_owned(), number("9"));
    usage.prompt_tokens_details = Some(details(r#"{"cached_tokens": 3}"#));
    backfill_cache_usage(&mut usage);
    assert_eq!(
        usage.extra.get("cache_read_input_tokens"),
        Some(&number("9"))
    );
}

#[test]
fn usage_backfill_defaults_to_zero_cached_details() {
    let mut usage = usage_with(1, 1);
    backfill_cache_usage(&mut usage);
    let counted = usage.prompt_tokens_details.expect("details are filled in");
    assert_eq!(counted.cached_tokens, Some(0));
    assert_eq!(
        usage.extra.get("cache_read_input_tokens"),
        Some(&number("0"))
    );
    assert_eq!(
        usage.extra.get("cache_creation_input_tokens"),
        Some(&number("0"))
    );
}

#[rstest]
#[case::cached(Some(r#"{"cached_tokens": 3}"#), &["cached_tokens"], 3)]
#[case::zero_is_skipped(
    Some(r#"{"cached_tokens": 0, "cache_write_tokens": 2}"#),
    &["cached_tokens", "cache_write_tokens"],
    2
)]
#[case::absent(None, &["cached_tokens"], 0)]
fn usage_cache_token_count_skips_zero_and_missing(
    #[case] source: Option<&str>,
    #[case] keys: &[&str],
    #[case] expected: i64,
) {
    let counted = source.map(details);
    assert_eq!(cache_token_count(counted.as_ref(), keys), expected);
}

#[test]
fn usage_coerce_mirrors_the_cache_write_and_creation_counters() {
    let counted = details(r#"{"cache_creation_tokens": 6}"#);
    let coerced = coerce_token_details(Some(&counted)).expect("the details survive");
    assert_eq!(coerced.cache_write_tokens, Some(6));
    assert_eq!(coerced.cache_creation_tokens, Some(6));
}

#[test]
fn usage_coerce_reads_the_nested_cache_creation_input_tokens() {
    let counted = details(r#"{"cache_creation_input_tokens": 5}"#);
    let coerced = coerce_token_details(Some(&counted)).expect("the details survive");
    assert_eq!(coerced.cache_write_tokens, Some(5));
    assert_eq!(coerced.cache_creation_tokens, Some(5));
}

#[rstest]
#[case::number("0.25", Some("0.25"))]
#[case::breakdown(r#"{"total_cost": 0.5}"#, Some("0.5"))]
#[case::bool("true", None)]
#[case::text(r#""0.5""#, None)]
#[case::other_breakdown(r#"{"other": 1}"#, None)]
#[case::null("null", None)]
fn usage_resolve_cost_reads_numbers_and_breakdowns(
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    let cost: JsonValue = parse(source);
    let wanted: Option<Decimal> = expected.map(|text| text.parse().expect("a decimal"));
    assert_eq!(resolve_provider_reported_cost(&cost), wanted);
}

#[test]
fn usage_cost_reaches_the_hidden_headers_only_for_a_listed_provider() {
    let mut response = chunk("c", 1);
    let mut usage = usage_with(1, 1);
    usage.cost = Some(number("0.5"));
    response.usage = Some(usage);
    propagate_usage_cost(
        &mut response,
        Some("openrouter"),
        &["openrouter".to_owned()],
    );
    let headers = response
        .hidden_params
        .get("additional_headers")
        .expect("the headers are created");
    let JsonValue::Object(fields) = headers else {
        panic!("the headers are an object");
    };
    assert_eq!(
        fields.get("llm_provider-x-litellm-response-cost"),
        Some(&number("0.5"))
    );
}

#[test]
fn usage_cost_stays_off_a_provider_outside_the_list() {
    let mut response = chunk("c", 1);
    let mut usage = usage_with(1, 1);
    usage.cost = Some(number("1"));
    response.usage = Some(usage);
    propagate_usage_cost(&mut response, Some("openai"), &["openrouter".to_owned()]);
    assert!(!response.hidden_params.contains_key("additional_headers"));
}

#[test]
fn usage_record_usage_only_chunk_keeps_the_usage_without_choices() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    let mut response = chunk("c", 1);
    response.usage = Some(usage_with(1, 2));
    record_usage_only_chunk(&mut shaper, response);
    assert_eq!(shaper.chunks.len(), 1);
    assert!(shaper.chunks[0].choices.is_empty());
    assert!(shaper.chunks[0].usage.is_some());
}

#[test]
fn usage_record_usage_only_chunk_ignores_a_chunk_without_usage() {
    let mut shaper = shaper("gpt-4", Some("openai"));
    record_usage_only_chunk(&mut shaper, chunk("c", 1));
    assert!(shaper.chunks.is_empty());
}

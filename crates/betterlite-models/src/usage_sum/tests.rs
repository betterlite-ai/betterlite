#![cfg(test)]

use super::*;

fn empty_usage() -> Usage {
    Usage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    }
}

fn empty_prompt_details() -> PromptTokensDetails {
    PromptTokensDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: None,
        google_maps_grounding_requests: None,
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cache_write_tokens: None,
        cache_creation_tokens: None,
        cached_tokens_details: None,
    }
}

#[test]
fn usage_sum_preserves_alias_and_does_not_recompute_total() {
    let mut first_details = empty_prompt_details();
    first_details.cache_write_tokens = Some(4);
    first_details.cache_creation_tokens = Some(4);
    let mut first = empty_usage();
    first.prompt_tokens = 2;
    first.completion_tokens = 3;
    first.total_tokens = 99;
    first.prompt_tokens_details = Some(first_details);

    let mut second_details = empty_prompt_details();
    second_details.cache_write_tokens = Some(6);
    second_details.cache_creation_tokens = Some(6);
    let mut second = empty_usage();
    second.prompt_tokens = 5;
    second.completion_tokens = 7;
    second.total_tokens = 1;
    second.prompt_tokens_details = Some(second_details);

    let result = usage_sum(&[first, second]).expect("usage sums");
    assert_eq!(result.prompt_tokens, 7);
    assert_eq!(result.completion_tokens, 10);
    assert_eq!(result.total_tokens, 100);
    let details = result.prompt_tokens_details.expect("details");
    assert_eq!(details.cache_write_tokens, Some(10));
    assert_eq!(details.cache_creation_tokens, Some(10));
}

#[test]
fn usage_sum_merges_cached_modalities() {
    let mut first_details = empty_prompt_details();
    first_details.cached_tokens_details = Some(CachedTokensDetails {
        text_tokens: None,
        audio_tokens: Some(2),
        image_tokens: Some(0),
    });
    let mut first = empty_usage();
    first.prompt_tokens_details = Some(first_details);

    let mut second_details = empty_prompt_details();
    second_details.cached_tokens_details = Some(CachedTokensDetails {
        text_tokens: Some(3),
        audio_tokens: None,
        image_tokens: Some(4),
    });
    let mut second = empty_usage();
    second.prompt_tokens_details = Some(second_details);

    let result = usage_sum(&[first, second]).expect("usage sums");
    assert_eq!(
        result
            .prompt_tokens_details
            .expect("details")
            .cached_tokens_details,
        Some(CachedTokensDetails {
            text_tokens: Some(3),
            audio_tokens: Some(2),
            image_tokens: Some(4)
        })
    );
}

#[test]
fn usage_sum_projects_decimal_and_boolean_extras() {
    let mut first_extras = BTreeMap::new();
    first_extras.insert(
        "duration".to_owned(),
        UsageNumber::Decimal(Decimal::new(15, 1)),
    );
    first_extras.insert("flag".to_owned(), UsageNumber::Boolean(true));
    let mut first = empty_usage();
    first.numeric_extras = first_extras;

    let mut second_extras = BTreeMap::new();
    second_extras.insert(
        "duration".to_owned(),
        UsageNumber::Decimal(Decimal::new(25, 1)),
    );
    second_extras.insert("flag".to_owned(), UsageNumber::Boolean(false));
    let mut second = empty_usage();
    second.numeric_extras = second_extras;

    let projected = usage_sum(&[first, second])
        .expect("usage sums")
        .to_cost_usage();
    assert_eq!(projected.numeric_extras["duration"], Decimal::from(4_i32));
    assert_eq!(projected.numeric_extras["flag"], Decimal::from(1_i32));
}

#[test]
fn usage_sum_defaults_modality_counters_to_zero_when_only_cache_fields_are_reported() {
    let mut source_details = empty_prompt_details();
    source_details.cache_write_tokens = Some(4);
    let mut usage = empty_usage();
    usage.prompt_tokens_details = Some(source_details);

    let result = usage_sum(&[usage]).expect("usage sums");
    let details = result.prompt_tokens_details.expect("details");
    assert_eq!(details.text_tokens, Some(0));
    assert_eq!(details.audio_tokens, Some(0));
    assert_eq!(details.image_tokens, Some(0));
    assert_eq!(details.video_tokens, Some(0));
}

#[test]
fn usage_sum_mirrors_either_cache_alias_into_both_names() {
    let mut first_details = empty_prompt_details();
    first_details.cache_write_tokens = Some(4);
    let mut first = empty_usage();
    first.prompt_tokens_details = Some(first_details);

    let mut second_details = empty_prompt_details();
    second_details.cache_creation_tokens = Some(5);
    let mut second = empty_usage();
    second.prompt_tokens_details = Some(second_details);

    let result = usage_sum(&[first, second]).expect("usage sums");
    let details = result.prompt_tokens_details.expect("details");
    assert_eq!(details.cache_write_tokens, Some(9));
    assert_eq!(details.cache_creation_tokens, Some(9));
}

#[test]
fn usage_sum_leaves_never_reported_counters_absent() {
    let mut first_details = empty_prompt_details();
    first_details.cache_write_tokens = Some(1);
    let mut first = empty_usage();
    first.prompt_tokens_details = Some(first_details);

    let mut second_details = empty_prompt_details();
    second_details.cache_write_tokens = Some(1);
    let mut second = empty_usage();
    second.prompt_tokens_details = Some(second_details);

    let result = usage_sum(&[first, second]).expect("usage sums");
    let details = result.prompt_tokens_details.expect("details");
    assert_eq!(details.web_search_requests, None);
    assert_eq!(details.query_count, None);
    assert_eq!(details.video_length_seconds, None);
}

#[test]
fn usage_sum_accumulates_fractional_duration_counters() {
    let mut first_details = empty_prompt_details();
    first_details.audio_length_seconds = Some(Decimal::new(15, 1));
    let mut first = empty_usage();
    first.prompt_tokens_details = Some(first_details);

    let mut second_details = empty_prompt_details();
    second_details.audio_length_seconds = Some(Decimal::new(25, 1));
    let mut second = empty_usage();
    second.prompt_tokens_details = Some(second_details);

    let result = usage_sum(&[first, second]).expect("usage sums");
    let details = result.prompt_tokens_details.expect("details");
    assert_eq!(details.audio_length_seconds, Some(Decimal::from(4_i32)));
}

#[test]
fn usage_sum_counter_overflow_is_a_typed_error() {
    let mut first = empty_usage();
    first.prompt_tokens = i64::MAX;
    let mut second = empty_usage();
    second.prompt_tokens = 1;

    assert_eq!(
        usage_sum(&[first, second]),
        Err(UsageSumError::IntegerOverflow {
            field: "prompt_tokens".to_owned()
        })
    );
}

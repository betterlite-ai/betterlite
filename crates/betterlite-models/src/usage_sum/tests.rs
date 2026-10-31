#![cfg(test)]

use super::*;

#[test]
fn usage_sum_preserves_alias_and_does_not_recompute_total() {
    let mut first = Usage::default();
    first.prompt_tokens = Some(2);
    first.completion_tokens = Some(3);
    first.total_tokens = Some(99);
    first.prompt_tokens_details = Some(PromptTokensDetails::default());
    if let Some(details) = first.prompt_tokens_details.as_mut() {
        details.cache_write_tokens = Some(4);
        details.cache_creation_tokens = Some(4);
    }
    let mut second = Usage::default();
    second.prompt_tokens = Some(5);
    second.completion_tokens = Some(7);
    second.total_tokens = Some(1);
    second.prompt_tokens_details = Some(PromptTokensDetails::default());
    if let Some(details) = second.prompt_tokens_details.as_mut() {
        details.cache_write_tokens = Some(6);
        details.cache_creation_tokens = Some(6);
    }
    let result = usage_sum(&[first, second]).expect("usage sums");
    assert_eq!(result.prompt_tokens, Some(7));
    assert_eq!(result.completion_tokens, Some(10));
    assert_eq!(result.total_tokens, Some(100));
    let details = result.prompt_tokens_details.expect("details");
    assert_eq!(details.cache_write_tokens, Some(10));
    assert_eq!(details.cache_creation_tokens, Some(10));
}

#[test]
fn usage_sum_merges_cached_modalities() {
    let mut first = Usage::default();
    first.prompt_tokens_details = Some(PromptTokensDetails::default());
    if let Some(details) = first.prompt_tokens_details.as_mut() {
        details.cached_tokens_details = Some(CachedTokensDetails {
            text_tokens: None,
            audio_tokens: Some(2),
            image_tokens: Some(0),
        });
    }
    let mut second = Usage::default();
    second.prompt_tokens_details = Some(PromptTokensDetails::default());
    if let Some(details) = second.prompt_tokens_details.as_mut() {
        details.cached_tokens_details = Some(CachedTokensDetails {
            text_tokens: Some(3),
            audio_tokens: None,
            image_tokens: Some(4),
        });
    }
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
    let mut first = Usage::default();
    first.numeric_extras.insert(
        "duration".to_owned(),
        UsageNumber::Decimal(Decimal::new(15, 1)),
    );
    first
        .numeric_extras
        .insert("flag".to_owned(), UsageNumber::Boolean(true));
    let mut second = Usage::default();
    second.numeric_extras.insert(
        "duration".to_owned(),
        UsageNumber::Decimal(Decimal::new(25, 1)),
    );
    second
        .numeric_extras
        .insert("flag".to_owned(), UsageNumber::Boolean(false));
    let projected = usage_sum(&[first, second])
        .expect("usage sums")
        .to_cost_usage()
        .expect("projection");
    assert_eq!(projected.numeric_extras["duration"], Decimal::from(4));
    assert_eq!(projected.numeric_extras["flag"], Decimal::from(1));
}

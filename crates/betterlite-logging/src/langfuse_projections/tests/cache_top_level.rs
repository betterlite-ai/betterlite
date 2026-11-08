#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::cache_usage::cache_read_tokens;
use super::support::value;
use crate::fields::usage::Usage;

fn usage_with_top_level(tokens: &str) -> Usage {
    let mut usage = Usage::default();
    usage
        .extras
        .insert("cache_read_input_tokens".to_owned(), value(tokens));
    usage
}

fn with_cached_tokens(mut usage: Usage, cached_tokens: Json) -> Usage {
    let mut details = IndexMap::new();
    details.insert("cached_tokens".to_owned(), cached_tokens);
    usage.prompt_tokens_details = Some(details);
    usage
}

#[test]
fn cache_top_level_reads_the_anthropic_field() {
    assert_eq!(cache_read_tokens(&usage_with_top_level("7")), 7);
}

#[test]
fn cache_gemini_reads_cached_tokens_details() {
    let usage = with_cached_tokens(Usage::default(), value("5"));
    assert_eq!(cache_read_tokens(&usage), 5);
}

#[test]
fn cache_openai_details_override_a_zero_top_level() {
    let usage = with_cached_tokens(usage_with_top_level("0"), value("4"));
    assert_eq!(cache_read_tokens(&usage), 4);
}

#[test]
fn cache_fallback_keeps_the_top_level_when_details_are_not_positive() {
    let zero = with_cached_tokens(usage_with_top_level("9"), value("0"));
    assert_eq!(cache_read_tokens(&zero), 9);
    let null = with_cached_tokens(usage_with_top_level("9"), Json::Null);
    assert_eq!(cache_read_tokens(&null), 9);
}

#[test]
fn cache_fallback_truncates_a_gemini_float_after_the_positive_check() {
    let usage = with_cached_tokens(Usage::default(), value("2.5"));
    assert_eq!(cache_read_tokens(&usage), 2);
}

#[test]
fn cache_top_level_ignores_a_missing_field() {
    assert_eq!(cache_read_tokens(&Usage::default()), 0);
}

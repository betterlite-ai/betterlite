#![cfg(test)]

use rstest::rstest;
use serde_json::Value;

use super::{
    ConvertedUsage, GIGACHAT_BASE_URL, IdSource as _, TokenUsage, UuidIds, arguments_text,
    convert_usage, get_api_base,
};
use crate::openai_base_config::Object;

fn usage(
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    precached_prompt_tokens: Option<u64>,
) -> TokenUsage {
    TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        precached_prompt_tokens,
        other: Object::new(),
    }
}

#[rstest]
#[case::argument_over_env(Some("https://base"), Some("https://env"), "https://base")]
#[case::env_over_default(None, Some("https://env"), "https://env")]
#[case::default(None, None, GIGACHAT_BASE_URL)]
#[case::empty_env_is_unset(None, Some(""), GIGACHAT_BASE_URL)]
fn api_base_lookup(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(get_api_base(api_base, env_base), expected);
}

#[test]
fn an_empty_api_base_argument_wins_over_the_environment_base() {
    assert_eq!(get_api_base(Some(""), Some("https://env")), "");
}

#[test]
fn usage_adds_precached_tokens_to_the_prompt_and_the_total() {
    let converted = convert_usage(&usage(Some(10), Some(4), Some(14), Some(6)));
    assert_eq!(
        converted,
        ConvertedUsage {
            prompt_tokens: 16,
            completion_tokens: 4,
            total_tokens: 20,
            cached_tokens: Some(6),
        }
    );
}

#[test]
fn usage_without_precached_tokens_reports_no_cached_details() {
    let converted = convert_usage(&usage(Some(10), Some(4), Some(14), None));
    assert_eq!(converted.prompt_tokens, 10);
    assert_eq!(converted.total_tokens, 14);
    assert_eq!(converted.cached_tokens, None);
}

#[test]
fn zero_precached_tokens_report_no_cached_details() {
    let converted = convert_usage(&usage(Some(10), Some(4), Some(14), Some(0)));
    assert_eq!(converted.cached_tokens, None);
}

#[test]
fn every_missing_counter_reads_as_zero() {
    let converted = convert_usage(&usage(None, None, None, None));
    assert_eq!(
        converted,
        ConvertedUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: None,
        }
    );
}

#[test]
fn an_unread_usage_key_still_makes_the_block_present() {
    let mut present = usage(None, None, None, None);
    present
        .other
        .insert(String::from("unknown"), Value::from(1_i32));
    assert!(present.is_present());
    assert!(!usage(None, None, None, None).is_present());
}

#[rstest]
#[case::prompt(usage(Some(1), None, None, None))]
#[case::completion(usage(None, Some(1), None, None))]
#[case::total(usage(None, None, Some(1), None))]
#[case::precached(usage(None, None, None, Some(1)))]
fn any_single_counter_makes_the_block_present(#[case] counters: TokenUsage) {
    assert!(counters.is_present());
}

#[test]
fn a_prompt_counter_that_overflows_stays_at_the_largest_value() {
    let converted = convert_usage(&usage(Some(u64::MAX), None, None, Some(1)));
    assert_eq!(converted.prompt_tokens, u64::MAX);
    assert_eq!(converted.cached_tokens, Some(1));
}

#[test]
fn a_total_counter_that_overflows_stays_at_the_largest_value() {
    let converted = convert_usage(&usage(None, None, Some(u64::MAX), Some(1)));
    assert_eq!(converted.total_tokens, u64::MAX);
}

#[test]
fn a_usage_object_decodes_with_defaults_and_extra_keys() {
    let decoded: TokenUsage =
        serde_json::from_str(r#"{"prompt_tokens": 3, "precached_prompt_tokens": 2, "extra": 9}"#)
            .unwrap();
    assert_eq!(decoded.prompt_tokens, Some(3));
    assert_eq!(decoded.precached_prompt_tokens, Some(2));
    assert_eq!(decoded.completion_tokens, None);
    assert!(decoded.other.contains_key("extra"));
}

#[rstest]
#[case::object(Some(Value::Object(Object::from_iter([(String::from("a"), Value::from(1_i32))]))), "{\"a\":1}")]
#[case::text(Some(Value::String(String::from("plain"))), "plain")]
#[case::number(Some(Value::from(5_i32)), "5")]
#[case::missing(None, "{}")]
#[case::null(Some(Value::Null), "{}")]
fn arguments_become_the_text_litellm_writes(
    #[case] arguments: Option<Value>,
    #[case] expected: &str,
) {
    assert_eq!(arguments_text(arguments.as_ref()), expected);
}

#[test]
fn the_uuid_source_makes_random_version_four_identifiers() {
    let first = UuidIds.new_uuid();
    assert_eq!(first.get_version_num(), 4);
    assert_ne!(first, UuidIds.new_uuid());
}

#![cfg(test)]

use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::{
    CACHE_TTL_1H_SECONDS, CACHE_TTL_5M_SECONDS, TurnCacheFacts, turn_cache_facts, write_ttl_seconds,
};

fn usage(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn facts(usage_object: Option<&JsonObject>) -> TurnCacheFacts {
    turn_cache_facts(usage_object).unwrap()
}

#[test]
fn absent_usage_is_uncovered_and_touched() {
    assert_eq!(
        facts(None),
        TurnCacheFacts {
            covered: false,
            read_tokens: 0,
            write_ttl_seconds: None,
            touched: true,
        }
    );
}

#[test]
fn an_empty_usage_object_counts_as_absent() {
    assert_eq!(facts(Some(&usage("{}"))), facts(None));
}

#[test]
fn usage_without_cache_signals_is_covered_and_untouched() {
    assert_eq!(
        facts(Some(&usage(
            r#"{"prompt_tokens":10,"completion_tokens":4}"#
        ))),
        TurnCacheFacts {
            covered: true,
            read_tokens: 0,
            write_ttl_seconds: None,
            touched: false,
        }
    );
}

#[test]
fn a_top_level_cache_read_marks_the_turn_touched() {
    assert_eq!(
        facts(Some(&usage(r#"{"cache_read_input_tokens":5}"#))),
        TurnCacheFacts {
            covered: true,
            read_tokens: 5,
            write_ttl_seconds: None,
            touched: true,
        }
    );
}

#[test]
fn the_openai_cached_tokens_detail_is_read() {
    assert_eq!(
        facts(Some(&usage(
            r#"{"prompt_tokens_details":{"cached_tokens":7}}"#
        )))
        .read_tokens,
        7
    );
}

#[test]
fn an_unsplit_cache_write_uses_the_five_minute_default() {
    assert_eq!(
        facts(Some(&usage(r#"{"cache_creation_input_tokens":10}"#))),
        TurnCacheFacts {
            covered: true,
            read_tokens: 0,
            write_ttl_seconds: Some(CACHE_TTL_5M_SECONDS),
            touched: true,
        }
    );
    assert_eq!(
        facts(Some(&usage(
            r#"{"prompt_tokens_details":{"cache_write_tokens":4}}"#
        )))
        .write_ttl_seconds,
        Some(CACHE_TTL_5M_SECONDS)
    );
}

#[test]
fn a_split_one_hour_write_wins_over_the_unsplit_count() {
    assert_eq!(
        facts(Some(&usage(
            r#"{"cache_creation_input_tokens":10,"prompt_tokens_details":{"cache_creation_token_details":{"ephemeral_1h_input_tokens":10}}}"#
        )))
        .write_ttl_seconds,
        Some(CACHE_TTL_1H_SECONDS)
    );
}

#[test]
fn a_split_five_minute_write_wins_over_the_unsplit_count() {
    assert_eq!(
        facts(Some(&usage(
            r#"{"cache_creation_input_tokens":10,"prompt_tokens_details":{"cache_creation_token_details":{"ephemeral_5m_input_tokens":10}}}"#
        )))
        .write_ttl_seconds,
        Some(CACHE_TTL_5M_SECONDS)
    );
}

#[test]
fn a_truthful_one_hour_split_wins_over_a_truthful_five_minute_one() {
    assert_eq!(
        facts(Some(&usage(
            r#"{"prompt_tokens_details":{"cache_creation_token_details":{"ephemeral_1h_input_tokens":3,"ephemeral_5m_input_tokens":9}}}"#
        )))
        .write_ttl_seconds,
        Some(CACHE_TTL_1H_SECONDS)
    );
}

#[test]
fn a_zero_split_falls_through_to_the_next_reading() {
    assert_eq!(
        facts(Some(&usage(
            r#"{"prompt_tokens_details":{"cache_creation_token_details":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":4}}}"#
        )))
        .write_ttl_seconds,
        Some(CACHE_TTL_5M_SECONDS)
    );
    assert_eq!(
        facts(Some(&usage(
            r#"{"cache_creation_input_tokens":3,"prompt_tokens_details":{"cache_creation_token_details":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":0}}}"#
        )))
        .write_ttl_seconds,
        Some(CACHE_TTL_5M_SECONDS)
    );
}

#[rstest]
#[case::text(r#""split""#)]
#[case::list("[1,2]")]
#[case::null("null")]
#[case::number("3")]
fn a_non_mapping_ttl_split_is_ignored(#[case] details: &str) {
    let payload = format!(
        r#"{{"cache_creation_input_tokens":2,"prompt_tokens_details":{{"cache_creation_token_details":{details}}}}}"#
    );
    assert_eq!(
        facts(Some(&usage(&payload))).write_ttl_seconds,
        Some(CACHE_TTL_5M_SECONDS)
    );
}

#[test]
fn a_cache_read_without_a_write_reads_no_ttl() {
    assert_eq!(
        facts(Some(&usage(
            r#"{"cache_read_input_tokens":5,"prompt_tokens_details":{"cached_tokens":5}}"#
        )))
        .write_ttl_seconds,
        None
    );
}

#[test]
fn no_usage_reads_no_ttl_on_its_own() {
    assert_eq!(write_ttl_seconds(None).unwrap(), None);
    assert_eq!(write_ttl_seconds(Some(&usage("{}"))).unwrap(), None);
}

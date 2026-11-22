use rust_decimal::Decimal;

use crate::auth_cache::WriteOptions;
use crate::spend_reseed::values::SpendValueError;
use crate::spend_writes::error::SpendWriteError;
use crate::spend_writes::source::SourceSpendCache;

use super::support::{Fixture, KEY, OTHER_KEY, auth_cache, decimal, fixture, source_json};

async fn base_of(fx: &Fixture, keys: &[&str]) -> Decimal {
    fx.writes
        .get_source_cache_base_spend(keys)
        .await
        .expect("the source cache answers")
}

#[tokio::test]
async fn the_first_source_that_answers_is_the_base() {
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("{\"spend\": 12.5}"));
    fx.sources.put(OTHER_KEY, source_json("{\"spend\": 99}"));
    assert_eq!(base_of(&fx, &[OTHER_KEY, KEY]).await, decimal("99"));
    assert_eq!(base_of(&fx, &[KEY, OTHER_KEY]).await, decimal("12.5"));
}

#[tokio::test]
async fn a_source_without_a_value_falls_through_to_the_next_key() {
    let fx = fixture(false);
    fx.sources.put(OTHER_KEY, source_json("{\"spend\": 4}"));
    assert_eq!(base_of(&fx, &[KEY, OTHER_KEY]).await, decimal("4"));
}

#[tokio::test]
async fn no_source_at_all_answers_zero() {
    let fx = fixture(false);
    assert_eq!(base_of(&fx, &[KEY, OTHER_KEY]).await, decimal("0"));
    assert_eq!(base_of(&fx, &[]).await, decimal("0"));
}

#[tokio::test]
async fn a_non_object_source_answers_zero() {
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("\"text\""));
    fx.sources.put(OTHER_KEY, source_json("{\"spend\": 4}"));
    assert_eq!(base_of(&fx, &[KEY, OTHER_KEY]).await, decimal("0"));
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("7"));
    assert_eq!(base_of(&fx, &[KEY]).await, decimal("0"));
}

#[tokio::test]
async fn a_source_without_the_spend_field_answers_zero() {
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("{\"total\": 4}"));
    fx.sources.put(OTHER_KEY, source_json("{\"spend\": 4}"));
    assert_eq!(base_of(&fx, &[KEY, OTHER_KEY]).await, decimal("0"));
}

#[tokio::test]
async fn empty_source_values_answer_zero() {
    for spend in ["null", "false", "\"\"", "[]", "{}"] {
        let fx = fixture(false);
        let source = format!("{{\"spend\": {spend}}}");
        fx.sources.put(KEY, source_json(&source));
        assert_eq!(base_of(&fx, &[KEY]).await, decimal("0"));
    }
}

#[tokio::test]
async fn a_true_spend_counts_as_one() {
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("{\"spend\": true}"));
    assert_eq!(base_of(&fx, &[KEY]).await, decimal("1"));
}

#[tokio::test]
async fn a_numeric_text_spend_is_read() {
    let fx = fixture(false);
    fx.sources.put(KEY, source_json("{\"spend\": \"2.75\"}"));
    assert_eq!(base_of(&fx, &[KEY]).await, decimal("2.75"));
}

#[tokio::test]
async fn a_non_empty_list_or_object_spend_is_unreadable() {
    for spend in ["[1]", "{\"amount\": 1}"] {
        let fx = fixture(false);
        let source = format!("{{\"spend\": {spend}}}");
        fx.sources.put(KEY, source_json(&source));
        let error = fx
            .writes
            .get_source_cache_base_spend(&[KEY])
            .await
            .expect_err("an unreadable spend surfaces");
        assert!(matches!(
            error,
            SpendWriteError::Value(SpendValueError::NotNumeric {
                kind: "an unreadable cache value",
            })
        ));
    }
}

#[tokio::test]
async fn the_auth_cache_answers_the_stored_payload() {
    let cache = auth_cache();
    let stored = source_json("{\"spend\": 4.5}");
    cache
        .set(Some(KEY), &stored, WriteOptions::default())
        .expect("the auth cache stores the fixture");
    let expected = cache
        .get(KEY, false)
        .expect("the stored payload reads back");
    assert_eq!(SourceSpendCache::cached(&cache, KEY).await, Some(expected));
    assert_eq!(SourceSpendCache::cached(&cache, "spend:absent").await, None);
}

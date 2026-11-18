#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{
    AGGREGATION_THRESHOLD_ENV, DEFAULT_FLUSH_COUNT, DEFAULT_LOCK_TTL_ENV, DEFAULT_LOCK_TTL_SECONDS,
    FLUSH_COUNT_ENV, QUEUE_CAPACITY_ENV, SpendQueueError, SpendQueueSettings,
    decode_spend_queue_setting, load_spend_queue_settings,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(pairs: &[(&str, &str)]) -> SpendQueueSettings {
    load_spend_queue_settings(&environment(pairs)).unwrap()
}

#[test]
fn loads_spend_queue_settings() {
    let defaults = load(&[]);
    assert_eq!(defaults.queue_capacity, 1000);
    assert_eq!(defaults.aggregation_threshold, 800);
    assert_eq!(defaults.flush_count, 1000);
    assert_eq!(defaults.default_lock_ttl_seconds, 60);

    let overridden = load(&[
        (QUEUE_CAPACITY_ENV, "7"),
        (AGGREGATION_THRESHOLD_ENV, "9"),
        (FLUSH_COUNT_ENV, "3"),
        (DEFAULT_LOCK_TTL_ENV, "30"),
    ]);
    assert_eq!(overridden.queue_capacity, 7);
    assert_eq!(overridden.aggregation_threshold, 9);
    assert_eq!(overridden.flush_count, 3);
    assert_eq!(overridden.default_lock_ttl_seconds, 30);

    let derived = load(&[
        (QUEUE_CAPACITY_ENV, "2500"),
        (FLUSH_COUNT_ENV, ""),
        (DEFAULT_LOCK_TTL_ENV, ""),
    ]);
    assert_eq!(derived.aggregation_threshold, 2000);
    assert_eq!(derived.flush_count, DEFAULT_FLUSH_COUNT);
    assert_eq!(derived.default_lock_ttl_seconds, DEFAULT_LOCK_TTL_SECONDS);
}

#[rstest]
#[case::one("1", 0)]
#[case::five("5", 4)]
#[case::seven("7", 5)]
#[case::ten("10", 8)]
fn derives_the_aggregation_threshold_floor(#[case] capacity: &str, #[case] expected: i64) {
    assert_eq!(
        load(&[(QUEUE_CAPACITY_ENV, capacity)]).aggregation_threshold,
        expected
    );
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative("-3", -3)]
fn keeps_a_nonpositive_flush_count(#[case] value: &str, #[case] expected: i64) {
    assert_eq!(load(&[(FLUSH_COUNT_ENV, value)]).flush_count, expected);
}

#[rstest]
#[case::zero_capacity(QUEUE_CAPACITY_ENV, "0")]
#[case::negative_capacity(QUEUE_CAPACITY_ENV, "-1")]
#[case::zero_default_ttl(DEFAULT_LOCK_TTL_ENV, "0")]
#[case::negative_default_ttl(DEFAULT_LOCK_TTL_ENV, "-60")]
fn rejects_a_nonpositive_capacity_or_default_ttl(#[case] name: &str, #[case] value: &str) {
    let error = load_spend_queue_settings(&environment(&[(name, value)])).unwrap_err();
    assert!(error.to_string().contains(name), "{error}");
}

#[test]
fn rejects_an_overflowing_threshold_default() {
    let capacity = i64::MAX.to_string();
    let error = load_spend_queue_settings(&environment(&[(QUEUE_CAPACITY_ENV, capacity.as_str())]))
        .unwrap_err();
    assert!(error.to_string().contains(QUEUE_CAPACITY_ENV), "{error}");
}

#[rstest]
#[case::text("not-a-number")]
#[case::fraction("10.5")]
#[case::underscored_digits("1_0")]
#[case::beyond_i64("9223372036854775808")]
fn rejects_a_malformed_integer_setting(#[case] value: &str) {
    for name in [
        QUEUE_CAPACITY_ENV,
        AGGREGATION_THRESHOLD_ENV,
        FLUSH_COUNT_ENV,
        DEFAULT_LOCK_TTL_ENV,
    ] {
        let error = load_spend_queue_settings(&environment(&[(name, value)])).unwrap_err();
        assert!(error.to_string().contains(name), "{error}");
    }
}

#[test]
fn rejects_a_zero_capacity_and_keeps_a_zero_aggregation_threshold() {
    assert!(matches!(
        decode_spend_queue_setting(QUEUE_CAPACITY_ENV, "0"),
        Err(SpendQueueError::NotPositive { .. })
    ));
    assert!(decode_spend_queue_setting(DEFAULT_LOCK_TTL_ENV, "0").is_err());
    assert_eq!(
        decode_spend_queue_setting(AGGREGATION_THRESHOLD_ENV, "0").unwrap(),
        Some(0)
    );
}

#[test]
fn decodes_one_setting_at_a_time() {
    assert_eq!(
        decode_spend_queue_setting(QUEUE_CAPACITY_ENV, " 12 ").unwrap(),
        Some(12)
    );
    assert_eq!(
        decode_spend_queue_setting(QUEUE_CAPACITY_ENV, "").unwrap(),
        None
    );
    assert_eq!(
        decode_spend_queue_setting(FLUSH_COUNT_ENV, "-1").unwrap(),
        Some(-1)
    );
    assert!(decode_spend_queue_setting(AGGREGATION_THRESHOLD_ENV, "1.5").is_err());
}

#[rstest]
fn replays_the_spend_queue_setting_corpus(
    #[files("fuzz/corpus/spend_queue_setting/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return;
    };
    for name in [
        QUEUE_CAPACITY_ENV,
        AGGREGATION_THRESHOLD_ENV,
        FLUSH_COUNT_ENV,
        DEFAULT_LOCK_TTL_ENV,
    ] {
        drop(decode_spend_queue_setting(name, text));
    }
}

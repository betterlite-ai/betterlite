#![cfg(test)]

use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::team_window_reset::error::WindowResetError;
use crate::team_window_reset::source::WindowSource;
use crate::team_window_reset::tests::support::{
    CountingReconnect, FakeCounterCache, KEY_COLUMN, KEY_TABLE, TEAM_COLUMN, TEAM_TABLE,
    TestDurations, TestSchedule, WindowHarness, close_store, decimal, finish, insert_windows,
    moment, next_text, past, schema, seed_window_spend, window_json, window_spend_row, window_text,
};

const PAST_TEXT: &str = "2025-12-30T00:00:00+00:00";
const FRESH_TEXT: &str = "2026-01-01T00:10:00+00:00";

#[tokio::test]
async fn expired_window_settles_the_counter_and_rolls_the_spend_row() {
    let (fixture, store, client) = schema().await;
    let mut window = window_json("1h", Some("10"), Some(PAST_TEXT));
    insert_windows(&client, KEY_TABLE, KEY_COLUMN, "k1", &[window.clone()]).await;
    seed_window_spend(&client, "key", "k1", "1h", past(), 99.0).await;
    let cache = FakeCounterCache::with_counter("spend:key:k1:window:1h", Some(decimal("12")));
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        cache,
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.rollover = RolloverPolicy { enabled: true };
    let reset = harness
        .resetter()
        .reset_expired_window(WindowSource::Keys, "k1", &mut window)
        .await
        .unwrap();
    assert!(reset);
    let counter = "spend:key:k1:window:1h".to_owned();
    assert_eq!(harness.cache.read_keys(), vec![counter.clone()]);
    assert_eq!(harness.cache.memory_of(&counter), Some(decimal("2")));
    assert_eq!(harness.cache.redis_of(&counter), Some(decimal("2")));
    assert_eq!(window_text(&window, "reset_at"), Some(next_text()));
    assert_eq!(harness.durations.requested(), vec!["1h".to_owned()]);
    let (start, spend, updated_at) = window_spend_row(&client, "key", "k1", "1h").await.unwrap();
    assert_eq!(start, moment(0).naive_utc());
    assert_eq!(spend.to_bits(), 0.0_f64.to_bits());
    assert_eq!(updated_at, moment(0).naive_utc());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn team_windows_use_the_team_counter_prefix() {
    let (fixture, store, client) = schema().await;
    insert_windows(
        &client,
        TEAM_TABLE,
        TEAM_COLUMN,
        "t1",
        &[window_json("2h", Some("5"), Some(PAST_TEXT))],
    )
    .await;
    seed_window_spend(&client, "team", "t1", "2h", past(), 7.0).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(7200)),
        FakeCounterCache::default(),
        TestDurations::fixed(7200),
        CountingReconnect::new(true),
    );
    let mut window = window_json("2h", Some("5"), Some(PAST_TEXT));
    let reset = harness
        .resetter()
        .reset_expired_window(WindowSource::Teams, "t1", &mut window)
        .await
        .unwrap();
    assert!(reset);
    assert_eq!(
        harness.cache.memory_of("spend:team:t1:window:2h"),
        Some(decimal("0"))
    );
    let (start, spend, _) = window_spend_row(&client, "team", "t1", "2h").await.unwrap();
    assert_eq!(start, moment(0).naive_utc());
    assert_eq!(spend.to_bits(), 0.0_f64.to_bits());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn window_with_a_future_reset_time_is_untouched() {
    let (fixture, store, client) = schema().await;
    let mut window = window_json("1h", Some("10"), Some(FRESH_TEXT));
    insert_windows(&client, KEY_TABLE, KEY_COLUMN, "k1", &[window.clone()]).await;
    seed_window_spend(&client, "key", "k1", "1h", past(), 99.0).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::with_counter("spend:key:k1:window:1h", Some(decimal("12"))),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    let reset = harness
        .resetter()
        .reset_expired_window(WindowSource::Keys, "k1", &mut window)
        .await
        .unwrap();
    assert!(!reset);
    assert!(harness.cache.read_keys().is_empty());
    assert_eq!(harness.cache.memory_of("spend:key:k1:window:1h"), None);
    assert_eq!(
        window_text(&window, "reset_at"),
        Some("2026-01-01T00:10:00+00:00".to_owned())
    );
    assert!(harness.durations.requested().is_empty());
    let (start, spend, _) = window_spend_row(&client, "key", "k1", "1h").await.unwrap();
    assert_eq!(start, past());
    assert_eq!(spend.to_bits(), 99.0_f64.to_bits());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn window_without_a_reset_time_is_untouched() {
    let (fixture, store, client) = schema().await;
    insert_windows(
        &client,
        KEY_TABLE,
        KEY_COLUMN,
        "k1",
        &[window_json("1h", Some("10"), None)],
    )
    .await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::with_counter("spend:key:k1:window:1h", Some(decimal("12"))),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    let mut window = window_json("1h", Some("10"), None);
    let reset = harness
        .resetter()
        .reset_expired_window(WindowSource::Keys, "k1", &mut window)
        .await
        .unwrap();
    assert!(!reset);
    assert!(harness.cache.read_keys().is_empty());
    assert_eq!(window_text(&window, "reset_at"), None);
    assert!(window_spend_row(&client, "key", "k1", "1h").await.is_none());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn expired_window_without_a_cap_resets_to_zero_without_a_read() {
    let (fixture, store, client) = schema().await;
    insert_windows(
        &client,
        KEY_TABLE,
        KEY_COLUMN,
        "k1",
        &[window_json("1h", None, Some(PAST_TEXT))],
    )
    .await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::with_counter("spend:key:k1:window:1h", Some(decimal("12"))),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.rollover = RolloverPolicy { enabled: true };
    let mut window = window_json("1h", None, Some(PAST_TEXT));
    let reset = harness
        .resetter()
        .reset_expired_window(WindowSource::Keys, "k1", &mut window)
        .await
        .unwrap();
    assert!(reset);
    assert!(harness.cache.read_keys().is_empty());
    assert_eq!(
        harness.cache.memory_of("spend:key:k1:window:1h"),
        Some(decimal("0"))
    );
    assert_eq!(
        harness.cache.redis_of("spend:key:k1:window:1h"),
        Some(decimal("0"))
    );
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn redis_failure_keeps_the_reset_and_the_memory_counter() {
    let (fixture, store, client) = schema().await;
    insert_windows(
        &client,
        KEY_TABLE,
        KEY_COLUMN,
        "k1",
        &[window_json("1h", Some("10"), Some(PAST_TEXT))],
    )
    .await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::failing_redis(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    let mut window = window_json("1h", Some("10"), Some(PAST_TEXT));
    let reset = harness
        .resetter()
        .reset_expired_window(WindowSource::Keys, "k1", &mut window)
        .await
        .unwrap();
    assert!(reset);
    assert_eq!(
        harness.cache.memory_of("spend:key:k1:window:1h"),
        Some(decimal("0"))
    );
    assert_eq!(harness.cache.redis_of("spend:key:k1:window:1h"), None);
    assert_eq!(window_text(&window, "reset_at"), Some(next_text()));
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn unparsable_duration_skips_only_the_roll() {
    let (fixture, store, client) = schema().await;
    insert_windows(
        &client,
        KEY_TABLE,
        KEY_COLUMN,
        "k1",
        &[window_json("1h", Some("10"), Some(PAST_TEXT))],
    )
    .await;
    seed_window_spend(&client, "key", "k1", "1h", past(), 99.0).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::failing_for(3600, "1h"),
        CountingReconnect::new(true),
    );
    let mut window = window_json("1h", Some("10"), Some(PAST_TEXT));
    let reset = harness
        .resetter()
        .reset_expired_window(WindowSource::Keys, "k1", &mut window)
        .await
        .unwrap();
    assert!(reset);
    assert_eq!(window_text(&window, "reset_at"), Some(next_text()));
    assert_eq!(harness.durations.requested(), vec!["1h".to_owned()]);
    let (start, spend, _) = window_spend_row(&client, "key", "k1", "1h").await.unwrap();
    assert_eq!(start, past());
    assert_eq!(spend.to_bits(), 99.0_f64.to_bits());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn calendar_overflow_skips_only_the_roll() {
    let (fixture, store, client) = schema().await;
    insert_windows(
        &client,
        KEY_TABLE,
        KEY_COLUMN,
        "k1",
        &[window_json("1h", Some("10"), Some(PAST_TEXT))],
    )
    .await;
    seed_window_spend(&client, "key", "k1", "1h", past(), 99.0).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(10_000_000_000_000_i64),
        CountingReconnect::new(true),
    );
    let mut window = window_json("1h", Some("10"), Some(PAST_TEXT));
    let reset = harness
        .resetter()
        .reset_expired_window(WindowSource::Keys, "k1", &mut window)
        .await
        .unwrap();
    assert!(reset);
    assert_eq!(window_text(&window, "reset_at"), Some(next_text()));
    let (start, spend, _) = window_spend_row(&client, "key", "k1", "1h").await.unwrap();
    assert_eq!(start, past());
    assert_eq!(spend.to_bits(), 99.0_f64.to_bits());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn refused_reset_time_leaves_the_window_unchanged() {
    let (fixture, store, client) = schema().await;
    insert_windows(
        &client,
        KEY_TABLE,
        KEY_COLUMN,
        "k1",
        &[window_json("1h", Some("10"), Some(PAST_TEXT))],
    )
    .await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::failing_for(moment(3600), "1h", "no such unit"),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    let mut window = window_json("1h", Some("10"), Some(PAST_TEXT));
    let error = harness
        .resetter()
        .reset_expired_window(WindowSource::Keys, "k1", &mut window)
        .await
        .unwrap_err();
    assert!(matches!(error, WindowResetError::Schedule { .. }));
    assert_eq!(window_text(&window, "reset_at"), Some(PAST_TEXT.to_owned()));
    assert_eq!(
        harness.cache.memory_of("spend:key:k1:window:1h"),
        Some(decimal("0"))
    );
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

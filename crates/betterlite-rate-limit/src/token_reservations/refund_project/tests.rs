#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;
use rstest::rstest;

use crate::limiter_records::{CacheCounterValue, ReservationBackend};
use crate::token_reservations::tests::support::{
    RecordingScript, ReservationFixture, START, count, identity, increment_tuples, local_write,
    read, scope, text, tokens_key, window_key,
};

const ITPM: &str = "model_per_project_itpm";
const VALUE: &str = "p1:gpt-4";

#[tokio::test]
async fn refunds_the_guarded_counter_of_a_local_window() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(ITPM, VALUE), text(START)),
        (&tokens_key(ITPM, VALUE), count(300)),
    ]);
    let windows = BTreeSet::from([identity(
        &tokens_key(ITPM, VALUE),
        &START.to_string(),
        ReservationBackend::Local,
    )]);

    fixture
        .reservations()
        .refund_reserved_tokens(&[scope(ITPM, VALUE)], 100, &windows)
        .await
        .unwrap();

    assert_eq!(
        fixture.effects.reads(),
        vec![
            read(&window_key(ITPM, VALUE)),
            read(&tokens_key(ITPM, VALUE))
        ]
    );
    assert_eq!(
        fixture.effects.writes(),
        vec![local_write(&tokens_key(ITPM, VALUE), 200.0_f64, 60)]
    );
    assert!(fixture.effects.pipelines().is_empty());
}

#[rstest]
#[case::moved_on(Some(text(1_699_999_980)))]
#[case::missing(None)]
#[tokio::test]
async fn skips_the_counter_when_the_window_guard_fails(
    #[case] active_window: Option<CacheCounterValue>,
) {
    let window = window_key(ITPM, VALUE);
    let counter = tokens_key(ITPM, VALUE);
    let mut entries: Vec<(&str, CacheCounterValue)> = vec![(&counter, count(300))];
    if let Some(active) = &active_window {
        entries.push((&window, active.clone()));
    }
    let fixture = ReservationFixture::seeded(&entries);
    let windows = BTreeSet::from([identity(
        &counter,
        &START.to_string(),
        ReservationBackend::Local,
    )]);

    fixture
        .reservations()
        .refund_reserved_tokens(&[scope(ITPM, VALUE)], 100, &windows)
        .await
        .unwrap();

    assert_eq!(fixture.effects.reads(), vec![read(&window)]);
    assert!(fixture.effects.writes().is_empty());
}

#[tokio::test]
async fn increments_unguarded_when_the_window_names_another_counter() {
    let fixture = ReservationFixture::seeded(&[]);
    let windows = BTreeSet::from([identity(
        &tokens_key("model_per_project_otpm", VALUE),
        &START.to_string(),
        ReservationBackend::Local,
    )]);

    fixture
        .reservations()
        .refund_reserved_tokens(&[scope(ITPM, VALUE)], 100, &windows)
        .await
        .unwrap();

    assert_eq!(
        increment_tuples(&fixture.effects.increments()),
        vec![(tokens_key(ITPM, VALUE), -100.0_f64, Some(60))]
    );
    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
}

#[tokio::test]
async fn sends_a_guard_script_call_for_a_redis_window() {
    let script = RecordingScript::new();
    let fixture = ReservationFixture::seeded(&[]).with_guarded_script(Arc::clone(&script));
    let windows = BTreeSet::from([identity(
        &tokens_key(ITPM, VALUE),
        &START.to_string(),
        ReservationBackend::Redis,
    )]);

    fixture
        .reservations()
        .refund_reserved_tokens(&[scope(ITPM, VALUE)], 100, &windows)
        .await
        .unwrap();

    assert_eq!(
        script.calls(),
        vec![(
            vec![window_key(ITPM, VALUE), tokens_key(ITPM, VALUE)],
            vec![
                RedisScalar::Text(START.to_string()),
                RedisScalar::Text(String::from("-100")),
                RedisScalar::Text(String::from("60")),
            ],
        )]
    );
    assert!(fixture.effects.increments().is_empty());
    assert!(fixture.effects.writes().is_empty());
}

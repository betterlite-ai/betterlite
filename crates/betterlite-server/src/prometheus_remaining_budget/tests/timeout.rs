use std::collections::BTreeMap;
use std::time::Duration;

use crate::prometheus_remaining_budget::fan_out::RemainingBudgetFanOut;

use super::support::{
    Behaviour, RealGauge, Recorder, capture_log, collectors_with, logged_text, metadata, request,
};

#[tokio::test(start_paused = true)]
async fn a_slow_lookup_returns_after_the_timeout_with_one_line() {
    let recorder = Recorder::new(BTreeMap::from([(
        "team",
        Behaviour::Slow(Duration::from_secs(60)),
    )]));
    let fan_out = RemainingBudgetFanOut::new(
        recorder.clone(),
        Duration::from_secs(5),
        &collectors_with(&[RealGauge::Team]),
    );
    let (guard, log) = capture_log();
    let started = tokio::time::Instant::now();
    fan_out
        .increment_remaining_budget_metrics(request(&metadata(&[])))
        .await;
    let elapsed = started.elapsed();
    drop(guard);
    assert_eq!(elapsed, Duration::from_secs(5));
    assert_eq!(recorder.calls().len(), 5);
    assert_eq!(recorder.finished(), ["key", "user", "org", "customer"]);
    let text = logged_text(&log);
    assert!(
        text.contains("per-request budget metric emission exceeded 5.0s under load"),
        "{text}"
    );
    assert!(!text.contains("Budget metric lookup"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn uses_the_configured_timeout() {
    let recorder = Recorder::new(BTreeMap::from([(
        "org",
        Behaviour::Slow(Duration::from_secs(60)),
    )]));
    let fan_out = RemainingBudgetFanOut::new(
        recorder.clone(),
        Duration::from_millis(1_500),
        &collectors_with(&[RealGauge::Team]),
    );
    let (guard, log) = capture_log();
    let started = tokio::time::Instant::now();
    fan_out
        .increment_remaining_budget_metrics(request(&metadata(&[])))
        .await;
    let elapsed = started.elapsed();
    drop(guard);
    assert_eq!(elapsed, Duration::from_millis(1_500));
    let text = logged_text(&log);
    assert!(text.contains("exceeded 1.5s under load"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn a_timeout_hides_a_failure_that_already_happened() {
    let recorder = Recorder::new(BTreeMap::from([
        ("key", Behaviour::Failing),
        ("team", Behaviour::Slow(Duration::from_secs(60))),
    ]));
    let fan_out = RemainingBudgetFanOut::new(
        recorder.clone(),
        Duration::from_secs(5),
        &collectors_with(&[RealGauge::Team]),
    );
    let (guard, log) = capture_log();
    fan_out
        .increment_remaining_budget_metrics(request(&metadata(&[])))
        .await;
    drop(guard);
    assert_eq!(recorder.finished(), ["user", "org", "customer"]);
    let text = logged_text(&log);
    assert!(text.contains("exceeded 5.0s under load"), "{text}");
    assert!(!text.contains("Budget metric lookup"), "{text}");
}

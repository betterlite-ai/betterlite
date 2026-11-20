#![cfg(test)]

use std::time::Duration;

use super::concurrency::{DispatchMode, run_probes};
use super::probe::ProbeOutcome;
use super::support::{FakeProvider, Reply, deployment, prepare_all, with_timeout};

fn answered_model(outcome: &ProbeOutcome) -> String {
    match outcome {
        ProbeOutcome::Healthy(response) => response
            .remaining_requests
            .clone()
            .unwrap_or_else(|| "no header".to_owned()),
        ProbeOutcome::Failed(failure) => failure.message.clone(),
        ProbeOutcome::Raised(failure) => failure.message.clone(),
    }
}

#[tokio::test(start_paused = true)]
async fn results_keep_the_input_order_whatever_the_completion_order() {
    let rows = [
        deployment("a", "a-id", r#"{"model":"openai/a"}"#),
        deployment("b", "b-id", r#"{"model":"openai/b"}"#),
        deployment("c", "c-id", r#"{"model":"openai/c"}"#),
    ];
    let prepared = prepare_all(&rows);
    let provider = FakeProvider::new()
        .with_reply(
            "openai/a",
            Reply::DelayedSuccess(Duration::from_millis(300)),
        )
        .with_reply(
            "openai/b",
            Reply::DelayedSuccess(Duration::from_millis(100)),
        )
        .with_reply(
            "openai/c",
            Reply::DelayedSuccess(Duration::from_millis(200)),
        );
    let (outcomes, mode, peak) = run_probes(&prepared, &provider, Some(3)).await;
    assert_eq!(mode, DispatchMode::Bounded);
    assert_eq!(peak, 3);
    let answered: Vec<String> = outcomes.iter().map(answered_model).collect();
    assert_eq!(
        answered,
        vec![
            "openai/a".to_owned(),
            "openai/b".to_owned(),
            "openai/c".to_owned()
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_limit_caps_the_probes_in_flight() {
    let rows = [
        deployment("a", "a-id", r#"{"model":"openai/a"}"#),
        deployment("b", "b-id", r#"{"model":"openai/b"}"#),
        deployment("c", "c-id", r#"{"model":"openai/c"}"#),
        deployment("d", "d-id", r#"{"model":"openai/d"}"#),
    ];
    let prepared = prepare_all(&rows);
    let provider = FakeProvider::new()
        .with_reply(
            "openai/a",
            Reply::DelayedSuccess(Duration::from_millis(100)),
        )
        .with_reply(
            "openai/b",
            Reply::DelayedSuccess(Duration::from_millis(100)),
        )
        .with_reply(
            "openai/c",
            Reply::DelayedSuccess(Duration::from_millis(100)),
        )
        .with_reply(
            "openai/d",
            Reply::DelayedSuccess(Duration::from_millis(100)),
        );
    let (outcomes, mode, peak) = run_probes(&prepared, &provider, Some(2)).await;
    assert_eq!(mode, DispatchMode::Bounded);
    assert_eq!(peak, 2);
    assert_eq!(provider.peak(), 2);
    assert_eq!(outcomes.len(), 4);
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ProbeOutcome::Healthy(_)))
    );
}

#[tokio::test(start_paused = true)]
async fn a_limit_above_the_input_still_reports_every_probe() {
    let rows = [
        deployment("a", "a-id", r#"{"model":"openai/a"}"#),
        deployment("b", "b-id", r#"{"model":"openai/b"}"#),
    ];
    let prepared = prepare_all(&rows);
    let provider = FakeProvider::new();
    let (outcomes, mode, peak) = run_probes(&prepared, &provider, Some(9)).await;
    assert_eq!(mode, DispatchMode::Bounded);
    assert_eq!(peak, 2);
    assert_eq!(outcomes.len(), 2);
}

async fn assert_unbounded(limit: Option<i64>) {
    let rows = [
        deployment("a", "a-id", r#"{"model":"openai/a"}"#),
        deployment("b", "b-id", r#"{"model":"openai/b"}"#),
        deployment("c", "c-id", r#"{"model":"openai/c"}"#),
    ];
    let prepared = prepare_all(&rows);
    let provider = FakeProvider::new();
    let (outcomes, mode, peak) = run_probes(&prepared, &provider, limit).await;
    assert_eq!(mode, DispatchMode::Unbounded);
    assert_eq!(peak, 3);
    assert_eq!(outcomes.len(), 3);
}

#[tokio::test]
async fn a_missing_limit_gathers_everything() {
    assert_unbounded(None).await;
}

#[tokio::test]
async fn a_zero_limit_gathers_everything() {
    assert_unbounded(Some(0)).await;
}

#[tokio::test]
async fn a_negative_limit_gathers_everything() {
    assert_unbounded(Some(-3)).await;
}

#[tokio::test(start_paused = true)]
async fn a_timeout_leaves_the_sibling_probe_running() {
    let rows = [
        with_timeout(
            deployment("slow", "slow-id", r#"{"model":"openai/slow"}"#),
            1.0,
        ),
        with_timeout(
            deployment("patient", "patient-id", r#"{"model":"openai/patient"}"#),
            30.0,
        ),
    ];
    let prepared = prepare_all(&rows);
    let provider = FakeProvider::new()
        .with_reply(
            "openai/slow",
            Reply::DelayedSuccess(Duration::from_secs(10)),
        )
        .with_reply(
            "openai/patient",
            Reply::DelayedSuccess(Duration::from_secs(2)),
        );
    let (outcomes, _mode, _peak) = run_probes(&prepared, &provider, Some(2)).await;
    match outcomes.first().expect("the slow probe answered") {
        ProbeOutcome::Failed(failure) => {
            assert_eq!(failure.status_code, 408);
            assert_eq!(failure.message, "Timeout exceeded");
        }
        ProbeOutcome::Healthy(_) => panic!("the slow probe must time out"),
        ProbeOutcome::Raised(_) => {
            panic!("a timeout answers with an error dict, not a raised probe")
        }
    }
    assert!(matches!(outcomes.get(1), Some(ProbeOutcome::Healthy(_))));
    assert_eq!(
        provider.calls(),
        vec!["openai/slow".to_owned(), "openai/patient".to_owned()]
    );
}

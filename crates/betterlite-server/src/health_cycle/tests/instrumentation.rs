use std::time::{Duration, Instant};

use betterlite_router::model_catalog_lookup::error::LookupError;
use rstest::rstest;

use super::cycle::finish_cycle;
use super::endpoints::HealthCheckResult;
use super::error::HealthCycleError;
use super::support::{
    FakeModels, FakeProvider, Reply, capture_logs, context, cycle, deployment, object,
    plain_request, policy, text,
};
use super::telemetry::{
    InstrumentationContext, cycle_failed, cycle_skipped, elapsed_ms, option_label,
};
use crate::health::router::error::RouterHealthError;

#[tokio::test]
async fn a_bounded_run_logs_its_limit_and_peak() {
    let (buffer, _guard) = capture_logs();
    let rows = [
        deployment("a", "a-id", r#"{"model":"openai/a"}"#),
        deployment("b", "b-id", r#"{"model":"openai/b"}"#),
    ];
    let context = context("cycle-2", "background");
    let mut request = plain_request(&rows);
    request.instrumentation = Some(&context);
    request.max_concurrency = Some(1);
    let provider = FakeProvider::new();
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    let logs = buffer.text();
    assert!(logs.contains("mode=bounded"), "{logs}");
    assert!(logs.contains("max_concurrency=1"), "{logs}");
    assert!(logs.contains("peak_in_flight=1"), "{logs}");
    assert!(logs.contains("health_check_cycle_start"), "{logs}");
    assert!(logs.contains("health_check_cycle_complete"), "{logs}");
}

#[tokio::test]
async fn a_disabled_run_logs_nothing() {
    let (buffer, _guard) = capture_logs();
    let rows = [deployment("a", "a-id", r#"{"model":"openai/a"}"#)];
    let provider = FakeProvider::new();
    cycle(
        plain_request(&rows),
        &provider,
        &FakeModels::default(),
        &policy(),
    )
    .await
    .expect("the cycle runs");
    assert_eq!(buffer.text(), "");
}

#[tokio::test]
async fn the_lines_hold_no_secret_provider_address_or_deployment_id() {
    let (buffer, _guard) = capture_logs();
    let rows = [deployment(
        "leaf",
        "secret-leaf-id",
        r#"{"model":"openai/leaf","api_key":"sk-secret-key","api_base":"https://internal.invalid"}"#,
    )];
    let context = context("cycle-4", "background");
    let mut request = plain_request(&rows);
    request.instrumentation = Some(&context);
    let provider = FakeProvider::new().with_reply(
        "openai/leaf",
        Reply::Failure {
            status: 500,
            message: "boom",
        },
    );
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    let logs = buffer.text();
    assert!(!logs.contains("sk-secret-key"), "{logs}");
    assert!(!logs.contains("internal.invalid"), "{logs}");
    assert!(!logs.contains("secret-leaf-id"), "{logs}");
    assert!(logs.contains("health_check_cycle_complete"), "{logs}");
}

#[test]
fn the_default_context_is_disabled_with_unknown_identity() {
    let context = InstrumentationContext::default();
    assert!(!context.enabled);
    assert_eq!(context.cycle_id, "unknown");
    assert_eq!(context.source, "unknown");
}

#[test]
fn a_cycle_result_logs_its_completion() {
    let (buffer, _guard) = capture_logs();
    let context = context("cycle-12", "background");
    let mut result = HealthCheckResult::default();
    result
        .healthy_endpoints
        .push(object(vec![("model_id", text("a"))]));
    result
        .unhealthy_endpoints
        .push(object(vec![("model_id", text("b"))]));
    result
        .unhealthy_endpoints
        .push(object(vec![("model_id", text("c"))]));
    let returned = finish_cycle(Ok(result), &context, 3, Instant::now())
        .expect("a completed cycle passes its result through");
    assert_eq!(returned.healthy_endpoints.len(), 1);
    let logs = buffer.text();
    assert!(logs.contains("health_check_cycle_complete"), "{logs}");
    assert!(logs.contains("model_count=3"), "{logs}");
    assert!(logs.contains("healthy_count=1"), "{logs}");
    assert!(logs.contains("unhealthy_count=2"), "{logs}");
}

#[test]
fn a_failed_cycle_keeps_its_error_after_logging() {
    let (buffer, _guard) = capture_logs();
    let context = context("cycle-13", "background");
    let failure = HealthCycleError::Router(RouterHealthError::Lookup(LookupError::InvalidRow {
        index: 4,
    }));
    let error = finish_cycle(Err(failure), &context, 2, Instant::now())
        .expect_err("a failed cycle returns its error");
    assert!(matches!(error, HealthCycleError::Router(_)));
    let logs = buffer.text();
    assert!(logs.contains("health_check_cycle_failed"), "{logs}");
    assert!(logs.contains("model_count=2"), "{logs}");
}

#[test]
fn a_failed_cycle_logs_its_error() {
    let (buffer, _guard) = capture_logs();
    let context = context("cycle-9", "background");
    cycle_failed(&context, 2, Duration::from_millis(5), "lookup failed");
    let logs = buffer.text();
    assert!(logs.contains("health_check_cycle_failed"), "{logs}");
    assert!(logs.contains("model_count=2"), "{logs}");
    assert!(logs.contains("lookup failed"), "{logs}");
    assert!(logs.contains("source=background"), "{logs}");
}

#[test]
fn a_skipped_cycle_logs_its_reason() {
    let (buffer, _guard) = capture_logs();
    let context = context("cycle-10", "background");
    cycle_skipped(&context, "no_models_after_filter");
    let logs = buffer.text();
    assert!(logs.contains("health_check_cycle_skipped"), "{logs}");
    assert!(logs.contains("reason=no_models_after_filter"), "{logs}");
}

#[test]
fn a_disabled_context_silences_the_cycle_lines() {
    let (buffer, _guard) = capture_logs();
    let context = InstrumentationContext::default();
    cycle_skipped(&context, "no_models");
    cycle_failed(&context, 1, Duration::ZERO, "boom");
    assert_eq!(buffer.text(), "");
}

#[test]
fn elapsed_milliseconds_are_a_float() {
    assert_eq!(
        elapsed_ms(Duration::from_millis(1500)).to_bits(),
        1500.0_f64.to_bits()
    );
    assert_eq!(elapsed_ms(Duration::ZERO).to_bits(), 0.0_f64.to_bits());
}

#[rstest]
#[case::absent(None, "None")]
#[case::positive(Some(4), "4")]
#[case::negative(Some(-2), "-2")]
fn the_concurrency_label_reads_like_percent_s(#[case] value: Option<i64>, #[case] expected: &str) {
    assert_eq!(option_label(value), expected);
}

#![cfg(test)]

use opentelemetry::trace::StatusCode;

use betterlite_logging::otel::v2_callback::registered;
use betterlite_logging::otel::v2_callback::registry::V2Registry;

use super::support::{attribute, error, event_names, harness, settings};

#[test]
fn dropping_the_phase_span_ends_it() {
    let harness = harness(settings());
    let phase = harness.callback.start_phase_span("auth");
    assert!(phase.span.is_recording());
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
    assert_eq!(spans[0].status.code, StatusCode::Unset);
}

#[test]
fn recording_an_error_stamps_the_status_and_one_event() {
    let harness = harness(settings());
    let phase = harness.callback.start_phase_span("auth");
    phase.record_error(&error("boom"));
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].status.code, StatusCode::Error);
    assert_eq!(event_names(&spans[0]), vec!["exception".to_owned()]);
    assert_eq!(
        attribute(&spans[0], "error.message").as_deref(),
        Some("boom")
    );
}

#[test]
fn recording_an_error_on_an_ended_span_changes_nothing() {
    let harness = harness(settings());
    let phase = harness.callback.start_phase_span("auth");
    phase.span.end();
    phase.record_error(&error("boom"));
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].status.code, StatusCode::Unset);
    assert!(spans[0].events.is_empty());
}

#[test]
fn the_module_entry_returns_none_without_a_logger() {
    let registry = V2Registry::default();
    assert!(registered::phase_span(&registry, "auth").is_none());
}

#[test]
fn the_module_entry_routes_to_the_registered_logger() {
    let harness = harness(settings());
    let phase = registered::phase_span(&harness.registry, "auth").expect("registered");
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
}

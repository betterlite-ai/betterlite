#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::{
    CAPTURE_MESSAGE_CONTENT_ENV, CaptureMode, CapturePolicy, EVENT_ONLY, LoggingFlags, NO_CONTENT,
    SPAN_AND_EVENT, SPAN_ONLY,
};
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};

fn base_config() -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter: Exporter::Console,
        endpoint: None,
        headers: None,
        signals: Signals::default(),
        service_name: String::new(),
        deployment_environment: String::new(),
        model_id: String::new(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

fn policy(capture: Option<&str>, environment: &BTreeMap<String, String>) -> CapturePolicy {
    let mut config = base_config();
    config.capture_message_content = capture.map(str::to_owned);
    CapturePolicy::from_init_state(&config, environment)
}

fn env(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
}

#[rstest]
#[case::true_alias("TRUE", CaptureMode::EventOnly)]
#[case::lowercase_true_alias("true", CaptureMode::EventOnly)]
#[case::one_alias("1", CaptureMode::EventOnly)]
#[case::false_alias("FALSE", CaptureMode::NoContent)]
#[case::zero_alias("0", CaptureMode::NoContent)]
#[case::no_content("NO_CONTENT", CaptureMode::NoContent)]
#[case::span_only("SPAN_ONLY", CaptureMode::SpanOnly)]
#[case::event_only("EVENT_ONLY", CaptureMode::EventOnly)]
#[case::span_and_event("SPAN_AND_EVENT", CaptureMode::SpanAndEvent)]
#[case::lowercase_name("span_only", CaptureMode::SpanOnly)]
#[case::mixed_case_name("Span_And_Event", CaptureMode::SpanAndEvent)]
#[case::padded_name_is_unknown("span_only ", CaptureMode::SpanAndEvent)]
#[case::unknown_text("turbo", CaptureMode::SpanAndEvent)]
fn sampled_modes_map_like_litellm(#[case] text: &str, #[case] expected: CaptureMode) {
    let policy = policy(Some(text), &BTreeMap::new());
    assert_eq!(
        policy.resolve(LoggingFlags {
            turn_off_message_logging: false,
            message_logging: true
        }),
        expected
    );
}

#[test]
fn a_non_empty_config_value_beats_the_env_var() {
    let environment = env(CAPTURE_MESSAGE_CONTENT_ENV, "SPAN_AND_EVENT");
    let policy = policy(Some("NO_CONTENT"), &environment);
    assert_eq!(
        policy.resolve(LoggingFlags {
            turn_off_message_logging: false,
            message_logging: true
        }),
        CaptureMode::NoContent
    );
}

#[test]
fn an_invalid_config_value_does_not_fall_through_to_the_env_var() {
    let environment = env(CAPTURE_MESSAGE_CONTENT_ENV, "SPAN_ONLY");
    let policy = policy(Some("bogus"), &environment);
    assert_eq!(
        policy.resolve(LoggingFlags {
            turn_off_message_logging: false,
            message_logging: true
        }),
        CaptureMode::SpanAndEvent
    );
}

#[test]
fn an_empty_config_value_uses_the_env_var() {
    let environment = env(CAPTURE_MESSAGE_CONTENT_ENV, "EVENT_ONLY");
    let policy = policy(Some(""), &environment);
    assert_eq!(
        policy.resolve(LoggingFlags {
            turn_off_message_logging: false,
            message_logging: false
        }),
        CaptureMode::EventOnly
    );
}

#[test]
fn the_kill_switch_beats_an_explicit_mode() {
    let policy = policy(Some("SPAN_AND_EVENT"), &BTreeMap::new());
    assert_eq!(
        policy.resolve(LoggingFlags {
            turn_off_message_logging: true,
            message_logging: true
        }),
        CaptureMode::NoContent
    );
}

#[test]
fn an_explicit_mode_beats_the_message_logging_flag() {
    let policy = policy(Some("SPAN_ONLY"), &BTreeMap::new());
    assert_eq!(
        policy.resolve(LoggingFlags {
            turn_off_message_logging: false,
            message_logging: false
        }),
        CaptureMode::SpanOnly
    );
}

#[test]
fn message_logging_decides_when_no_mode_is_explicit() {
    let policy = policy(None, &BTreeMap::new());
    assert_eq!(
        policy.resolve(LoggingFlags {
            turn_off_message_logging: false,
            message_logging: true
        }),
        CaptureMode::SpanAndEvent
    );
    assert_eq!(
        policy.resolve(LoggingFlags {
            turn_off_message_logging: false,
            message_logging: false
        }),
        CaptureMode::NoContent
    );
}

#[rstest]
#[case::no_content(CaptureMode::NoContent, false, false)]
#[case::span_only(CaptureMode::SpanOnly, true, false)]
#[case::event_only(CaptureMode::EventOnly, false, true)]
#[case::span_and_event(CaptureMode::SpanAndEvent, true, true)]
fn capture_predicates_match_the_modes(
    #[case] mode: CaptureMode,
    #[case] in_span: bool,
    #[case] in_event: bool,
) {
    assert_eq!(mode.capture_in_span(), in_span);
    assert_eq!(mode.capture_in_event(), in_event);
}

#[rstest]
#[case::no_content(CaptureMode::NoContent, NO_CONTENT)]
#[case::span_only(CaptureMode::SpanOnly, SPAN_ONLY)]
#[case::event_only(CaptureMode::EventOnly, EVENT_ONLY)]
#[case::span_and_event(CaptureMode::SpanAndEvent, SPAN_AND_EVENT)]
fn mode_names_match_litellm(#[case] mode: CaptureMode, #[case] expected: &str) {
    assert_eq!(mode.as_str(), expected);
}

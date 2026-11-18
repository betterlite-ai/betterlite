use betterlite_logging::rubrik_event_logging::error::EventPayloadError;
use betterlite_logging::rubrik_event_logging::event::EventKind;
use betterlite_logging::rubrik_event_logging::prepare::prepare_log_payload;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;
use tracing::Level;

use super::support::{Harness, ScriptedRandom, details, field, json, text};

const BASE: &str = r#"{"standard_logging_object":{"id":"chatcmpl-provider","status":"success","messages":[{"role":"user","content":"hi"}],"provider_detail":{"x":1}}}"#;

fn prepared(
    samples: &[f64],
    rate: f64,
    call_details: &str,
) -> Result<Option<Json>, EventPayloadError> {
    let fields = details(call_details);
    prepare_log_payload(
        &ScriptedRandom::new(samples),
        rate,
        &fields,
        EventKind::Success,
    )
    .map(|payload| payload.map(Json::Object))
}

fn kept(samples: &[f64], rate: f64, call_details: &str) -> Json {
    prepared(samples, rate, call_details)
        .expect("the payload is prepared")
        .expect("the draw is kept")
}

#[rstest]
#[case::below(0.1, 0.25)]
#[case::equal(0.25, 0.25)]
#[case::zero_rate(0.0, 0.0)]
#[case::full_rate(0.999_999, 1.0)]
fn a_draw_at_or_below_the_rate_keeps_the_event(#[case] sample: f64, #[case] rate: f64) {
    let payload = kept(&[sample], rate, BASE);
    assert_eq!(text(field(&payload, "id")), "chatcmpl-provider");
}

#[rstest]
#[case::above(0.500_000_1, 0.25)]
#[case::above_zero(0.000_001, 0.0)]
fn a_draw_above_the_rate_drops_the_event(#[case] sample: f64, #[case] rate: f64) {
    let outcome = prepared(&[sample], rate, BASE).expect("the payload is prepared");
    assert!(outcome.is_none());
}

#[test]
fn a_sampled_out_event_is_logged_with_its_kind() {
    let harness = Harness::new(&[], &[]);
    let fields = details(BASE);
    let outcome = prepare_log_payload(
        &ScriptedRandom::new(&[0.9]),
        0.5,
        &fields,
        EventKind::Failure,
    )
    .expect("the payload is prepared");
    assert!(outcome.is_none());
    let debug = harness.logs.with_level(Level::DEBUG);
    assert_eq!(debug.len(), 1);
    assert_eq!(debug[0].message, "Skipping Rubrik logging");
    assert_eq!(debug[0].event.as_deref(), Some("failure"));
}

#[test]
fn a_sampled_out_event_reads_no_standard_logging_object() {
    let outcome = prepared(&[0.9], 0.5, "{}").expect("the payload is prepared");
    assert!(outcome.is_none());
}

#[test]
fn an_absent_standard_logging_object_is_an_error() {
    let error = prepared(&[0.0], 1.0, "{}").expect_err("a missing object is an error");
    assert!(matches!(
        error,
        EventPayloadError::MissingStandardLoggingObject
    ));
}

#[rstest]
#[case::text(r#"{"standard_logging_object":"chatcmpl-provider"}"#)]
#[case::null(r#"{"standard_logging_object":null}"#)]
fn a_standard_logging_object_that_is_not_an_object_is_a_field_error(#[case] call_details: &str) {
    let error = prepared(&[0.0], 1.0, call_details).expect_err("a wrong shape is an error");
    assert!(matches!(
        error,
        EventPayloadError::FieldType {
            field: "standard_logging_object"
        }
    ));
}

#[test]
fn the_payload_is_a_deep_copy() {
    let fields = details(BASE);
    let mut payload = prepare_log_payload(
        &ScriptedRandom::new(&[0.0]),
        1.0,
        &fields,
        EventKind::Success,
    )
    .expect("the payload is prepared")
    .expect("the draw is kept");
    let Json::Object(detail) = payload
        .get_mut("provider_detail")
        .expect("the detail is copied")
    else {
        panic!("the provider detail is an object");
    };
    detail.insert("x".to_owned(), Json::Number(Number::from(2_u8)));
    let source = json(BASE);
    assert_eq!(
        field(field(&source, "standard_logging_object"), "provider_detail"),
        &json(r#"{"x":1}"#)
    );
}

#[test]
fn an_unknown_standard_field_is_kept() {
    let payload = kept(&[0.0], 1.0, BASE);
    assert_eq!(text(field(&payload, "status")), "success");
    assert_eq!(field(&payload, "provider_detail"), &json(r#"{"x":1}"#));
}

#[test]
fn the_call_id_replaces_the_provider_response_id() {
    let payload = kept(
        &[0.0],
        1.0,
        r#"{"litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider"}}"#,
    );
    assert_eq!(text(field(&payload, "id")), "call-1");
}

#[rstest]
#[case::absent(r#"{"standard_logging_object":{"id":"chatcmpl-provider"}}"#)]
#[case::empty(r#"{"litellm_call_id":"","standard_logging_object":{"id":"chatcmpl-provider"}}"#)]
#[case::null(r#"{"litellm_call_id":null,"standard_logging_object":{"id":"chatcmpl-provider"}}"#)]
fn a_call_id_that_is_unset_keeps_the_provider_response_id(#[case] call_details: &str) {
    let payload = kept(&[0.0], 1.0, call_details);
    assert_eq!(text(field(&payload, "id")), "chatcmpl-provider");
}

#[test]
fn a_wrong_typed_call_id_is_a_call_details_error() {
    let error = prepared(
        &[0.0],
        1.0,
        r#"{"litellm_call_id":5,"standard_logging_object":{"id":"chatcmpl-provider"}}"#,
    )
    .expect_err("a numeric call id is an error");
    assert!(matches!(error, EventPayloadError::CallDetails { .. }));
}

#[test]
fn the_system_prompt_is_prepended() {
    let payload = kept(
        &[0.0],
        1.0,
        r#"{"system":"be nice","standard_logging_object":{"messages":[{"role":"user","content":"hi"}]}}"#,
    );
    assert_eq!(
        field(&payload, "messages"),
        &json(r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#)
    );
}

#[rstest]
#[case::absent(BASE)]
#[case::empty(
    r#"{"system":"","standard_logging_object":{"messages":[{"role":"user","content":"hi"}]}}"#
)]
fn a_system_prompt_that_is_unset_leaves_the_messages_alone(#[case] call_details: &str) {
    let payload = kept(&[0.0], 1.0, call_details);
    assert_eq!(
        field(&payload, "messages"),
        &json(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[test]
fn a_system_prompt_without_messages_adds_no_messages() {
    let payload = kept(
        &[0.0],
        1.0,
        r#"{"system":"be nice","standard_logging_object":{"id":"chatcmpl-provider"}}"#,
    );
    let Json::Object(fields) = payload else {
        panic!("the payload is an object");
    };
    assert!(!fields.contains_key("messages"));
}

#[test]
fn the_system_prompt_does_not_rewrite_the_shared_messages() {
    let fields = details(
        r#"{"system":"be nice","standard_logging_object":{"messages":[{"role":"user","content":"hi"}]}}"#,
    );
    let payload = prepare_log_payload(
        &ScriptedRandom::new(&[0.0]),
        1.0,
        &fields,
        EventKind::Success,
    )
    .expect("the payload is prepared")
    .expect("the draw is kept");
    assert_eq!(
        payload.get("messages"),
        Some(&json(
            r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#
        ))
    );
    assert_eq!(
        fields.get("standard_logging_object"),
        Some(&json(r#"{"messages":[{"role":"user","content":"hi"}]}"#))
    );
}

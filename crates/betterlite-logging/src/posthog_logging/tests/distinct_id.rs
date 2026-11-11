use betterlite_models::decisions::Json;

use crate::fields::payload::StandardLoggingPayload;
use crate::posthog_logging::event::create_event_payload;
use crate::posthog_logging::tests::support::{fixed_id, object, payload, text};

fn distinct_id(payload: &StandardLoggingPayload) -> String {
    create_event_payload(payload, fixed_id)
        .expect("the event builds")
        .distinct_id
}

#[test]
fn distinct_id_prefers_the_metadata_user_id() {
    let mut payload = payload();
    payload.end_user = Some("end-1".to_owned());
    payload.metadata.requester_metadata = Some(object(&[("user_id", text("user-1"))]));
    assert_eq!(distinct_id(&payload), "user-1");
}

#[test]
fn distinct_id_uses_the_end_user_without_a_user_id() {
    let mut payload = payload();
    payload.end_user = Some("end-1".to_owned());
    assert_eq!(distinct_id(&payload), "end-1");
}

#[test]
fn distinct_id_uses_the_trace_id_without_a_user_or_end_user() {
    assert_eq!(distinct_id(&payload()), "trace-1");
}

#[test]
fn distinct_id_falls_back_to_a_generated_id() {
    let mut payload = payload();
    payload.trace_id = String::new();
    assert_eq!(distinct_id(&payload), "generated-id");
}

#[test]
fn distinct_id_skips_an_empty_user_id() {
    let mut payload = payload();
    payload.end_user = Some("end-1".to_owned());
    payload.metadata.requester_metadata = Some(object(&[("user_id", text(""))]));
    assert_eq!(distinct_id(&payload), "end-1");
}

#[test]
fn distinct_id_skips_a_user_id_that_is_not_text() {
    let mut payload = payload();
    payload.end_user = Some("end-1".to_owned());
    payload.metadata.requester_metadata = Some(object(&[("user_id", Json::Bool(true))]));
    assert_eq!(distinct_id(&payload), "end-1");
}

#[test]
fn distinct_id_generation_produces_a_fresh_uuid() {
    let generate = || uuid::Uuid::new_v4().to_string();
    let first = generate();
    let second = generate();
    assert_eq!(first.len(), 36);
    assert_ne!(first, second);
}

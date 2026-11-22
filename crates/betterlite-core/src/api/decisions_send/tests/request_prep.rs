use std::sync::Mutex;

use super::support::*;
use crate::api::decisions_send::format::DecisionsOutput;

// request_prep: validate_environment, url, the body copy and the optional pre_call, in that order

#[test]
fn request_prep_runs_validate_url_and_pre_call_in_order() {
    let mut fixture = Fixture::new();
    let logging = Mutex::new(logging_state());
    let mut decisions = systemone_call("typesafe", Some(&logging));
    decisions
        .headers
        .insert("X-Custom".to_owned(), "one".to_owned());
    let output = fixture.send(&decisions).expect("the call answers");
    assert!(matches!(output, DecisionsOutput::SystemOne(_)));
    assert_eq!(
        fixture.recording.events.recorded(),
        expected(&[
            "validate",
            "url",
            "log",
            "sign",
            "select-sync",
            "send",
            "transform",
        ])
    );
    let validations = fixture.recording.seen_validations();
    assert_eq!(validations.len(), 1);
    assert_eq!(validations[0].model, "model-1");
    assert_eq!(validations[0].api_key.as_deref(), Some("test-key"));
    assert_eq!(
        validations[0].headers,
        vec![(String::from("X-Custom"), String::from("one"))]
    );
}

#[test]
fn request_prep_builds_url_from_api_base_and_model() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    assert_eq!(
        fixture.recording.seen_urls(),
        vec![(String::from("http://127.0.0.1:9"), String::from("model-1"))]
    );
}

#[test]
fn request_prep_skips_logging_without_a_logging_object() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let events = fixture.recording.events.recorded();
    assert!(!events.contains(&String::from("log")));
    assert_eq!(fixture.wire.requests().len(), 1);
}

#[test]
fn request_prep_logs_the_unsigned_headers_and_body_copy() {
    let mut fixture = Fixture::new();
    fixture.config.signature = Some(String::from("sig"));
    let logging = Mutex::new(logging_state());
    let decisions = systemone_call("typesafe", Some(&logging));
    fixture.send(&decisions).expect("the call answers");
    let signed = &fixture.recording.seen_signs()[0];
    assert!(signed.headers.contains(&(
        String::from("Content-Type"),
        String::from("application/json")
    )));
    assert!(signed.headers.contains(&(
        String::from("Authorization"),
        String::from("Bearer test-key")
    )));
    let seen = fixture
        .recording
        .seen_validations()
        .first()
        .cloned()
        .expect("the environment was validated");
    assert!(
        seen.headers.is_empty(),
        "pre_call must see the call's own headers, not the signed ones"
    );
}

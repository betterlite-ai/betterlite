#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use rstest::rstest;

use super::super::failure::failure_frame;
use super::super::types::{FailureKind, FailureParam, StreamFailure};
use super::{
    EndState, Outcome, body_of, chat_chunk, chunk_frame, collect, count, expected_chunk, frames,
    other_failure, plain_generator, recorded, recorded_outcomes, recording_generator, request,
    source,
};
use crate::stream_chunks::{DONE_FRAME, StreamItem};

fn payload(frame: &str) -> &str {
    frame
        .strip_prefix("data: ")
        .and_then(|rest| rest.strip_suffix("\n\n"))
        .expect("data frame")
}

fn error_frame_payload(failure: &StreamFailure) -> String {
    payload(&failure_frame(failure)).to_owned()
}

fn http_failure(message: &str) -> StreamFailure {
    let mut failure = other_failure(message);
    failure.kind = FailureKind::Http;
    failure
}

#[test]
fn failure_frame_missing_fields() {
    assert_eq!(
        error_frame_payload(&other_failure("boom")),
        r#"{"error":{"message":"boom","type":"None","param":"None","code":"500"}}"#
    );
}

#[rstest]
#[case::null(
    FailureParam::Null,
    r#"{"error":{"message":"boom","type":"None","param":null,"code":"500"}}"#
)]
#[case::text(
    FailureParam::Text(String::from("messages")),
    r#"{"error":{"message":"boom","type":"None","param":"messages","code":"500"}}"#
)]
fn failure_frame_param_shapes(#[case] param: FailureParam, #[case] expected: &str) {
    let mut failure = other_failure("boom");
    failure.param = param;
    assert_eq!(error_frame_payload(&failure), expected);
}

#[test]
fn failure_keeps_type_and_status() {
    let mut failure = other_failure("boom");
    failure.error_type = Some(String::from("AuthenticationError"));
    failure.param = FailureParam::Text(String::from("messages"));
    failure.status_code = Some(422);
    assert_eq!(
        error_frame_payload(&failure),
        r#"{"error":{"message":"boom","type":"AuthenticationError","param":"messages","code":"422"}}"#
    );
}

#[rstest]
#[case::no_healthy("No healthy deployment available", 500, "429")]
#[case::no_deployments("No deployments available for selected model", 500, "429")]
#[case::tags("Not allowed to access model due to tags configuration", 500, "401")]
#[case::status("connection reset", 503, "503")]
#[case::zero("connection reset", 0, "0")]
#[case::precedence(
    "No deployments available; Not allowed to access model due to tags configuration",
    401,
    "429"
)]
#[case::default_status("connection reset", 500, "500")]
fn failure_frame_code_rules(#[case] message: &str, #[case] status: u16, #[case] code: &str) {
    let mut failure = other_failure(message);
    failure.status_code = Some(status);
    assert!(error_frame_payload(&failure).contains(&format!("\"code\":\"{code}\"")));
}

#[test]
fn plain_failure_omits_done() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) = recording_generator(Arc::clone(&events), Arc::clone(&outcomes), None);
    let out = frames(body_of(
        &generator,
        source(vec![Err(other_failure("boom"))], JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(
        out,
        vec![String::from(
            "data: {\"error\":{\"message\":\"boom\",\"type\":\"None\",\"param\":\"None\",\"code\":\"500\"}}\n\n"
        )]
    );
    assert_eq!(count(&recorded(&events), "failure:boom"), 1);
    assert_eq!(
        recorded_outcomes(&outcomes),
        vec![Outcome {
            key: String::from("key-1"),
            state: EndState::Completed,
            has_request: false,
            has_snapshot: false,
        }]
    );
}

#[test]
fn http_failure_aborts_body() {
    let outcome_log = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) = recording_generator(
        Arc::new(Mutex::new(Vec::new())),
        Arc::clone(&outcome_log),
        None,
    );
    let out = collect(body_of(
        &generator,
        source(vec![Err(http_failure("boom"))], JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].as_ref().expect_err("io error").to_string(), "boom");
    let outcomes = recorded_outcomes(&outcome_log);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].state, EndState::Aborted);
}

#[rstest]
#[case::done(JsonValue::Null, 2)]
#[case::skipped(JsonValue::from(true), 1)]
fn error_state_failure_frames(#[case] skip: JsonValue, #[case] length: usize) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) = recording_generator(
        Arc::clone(&events),
        Arc::clone(&outcomes),
        Some(String::from("event: response.failed\n\n")),
    );
    let data = request(&[("_litellm_skip_openai_stream_done", skip)]);
    let out = frames(body_of(
        &generator,
        source(vec![Err(other_failure("boom"))], JsonObject::new()),
        data,
        true,
    ));
    assert_eq!(out.len(), length);
    assert_eq!(out[0], "event: response.failed\n\n");
    assert_eq!(
        out.last().expect("frame"),
        if length == 2 {
            DONE_FRAME
        } else {
            "event: response.failed\n\n"
        }
    );
    let log = recorded(&events);
    assert_eq!(count(&log, "new-state"), 1);
    assert_eq!(count(&log, "format"), 1);
    assert_eq!(count(&log, "mark"), 0);
    assert_eq!(recorded_outcomes(&outcomes)[0].state, EndState::Completed);
}

#[test]
fn http_error_state_emits_frame() {
    let (generator, _hooks) = recording_generator(
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
        Some(String::from("event: response.failed\n\n")),
    );
    let out = collect(body_of(
        &generator,
        source(vec![Err(http_failure("boom"))], JsonObject::new()),
        request(&[]),
        true,
    ));
    let texts: Vec<String> = out
        .into_iter()
        .map(|frame| String::from_utf8(frame.expect("frame").to_vec()).expect("utf8"))
        .collect();
    assert_eq!(
        texts,
        vec![
            String::from("event: response.failed\n\n"),
            DONE_FRAME.to_owned(),
        ]
    );
}

#[test]
fn error_state_done_only() {
    let out = frames(body_of(
        &plain_generator(),
        source(vec![Err(other_failure("boom"))], JsonObject::new()),
        request(&[]),
        true,
    ));
    assert_eq!(out, vec![DONE_FRAME.to_owned()]);
}

#[test]
fn failure_hook_error_aborts_body() {
    let outcome_log = Arc::new(Mutex::new(Vec::new()));
    let (generator, hooks) = recording_generator(
        Arc::new(Mutex::new(Vec::new())),
        Arc::clone(&outcome_log),
        None,
    );
    hooks.set_failure_fails(true);
    let out = collect(body_of(
        &generator,
        source(vec![Err(other_failure("boom"))], JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].as_ref().expect_err("io error").to_string(),
        "failure hook failed"
    );
    assert_eq!(recorded_outcomes(&outcome_log)[0].state, EndState::Aborted);
}

#[test]
fn only_data_frames_are_marked() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    let items = vec![
        Ok(chat_chunk("x")),
        Ok(StreamItem::Bytes(Bytes::from_static(b": ping"))),
        Ok(StreamItem::Text(String::from("data: failed"))),
    ];
    let out = frames(body_of(
        &generator,
        source(items, JsonObject::new()),
        request(&[]),
        true,
    ));
    assert_eq!(out.len(), 4);
    assert_eq!(out[0], chunk_frame(&expected_chunk("x", None)));
    assert_eq!(out[1], ": ping\n\n");
    assert_eq!(out[2], "data: failed");
    assert_eq!(out[3], DONE_FRAME);
    let log = recorded(&events);
    assert_eq!(count(&log, "mark"), 1);
    assert_eq!(count(&log, "observe"), 3);
}

#[test]
fn stripped_usage_is_not_observed() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    let out = frames(body_of(
        &generator,
        source(vec![Ok(super::usage_artifact())], JsonObject::new()),
        request(&[("_litellm_strip_stream_usage", JsonValue::from(true))]),
        true,
    ));
    assert_eq!(out, [DONE_FRAME]);
    assert_eq!(count(&recorded(&events), "observe"), 0);
    assert_eq!(count(&recorded(&events), "mark"), 0);
}

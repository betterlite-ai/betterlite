#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_error::facts::ErrorCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::super::failure::failure_frame;
use super::super::types::{ErrorStateSource, StreamErrorState, StreamFailure};
use super::{body_of, frames, object, other_failure, plain_generator, request, source};
use crate::stream_chunks::{DONE_FRAME, StreamItem};

struct FailureCapture(Arc<Mutex<Vec<StreamFailure>>>);

impl ErrorStateSource for FailureCapture {
    fn new_state(&self) -> Box<dyn StreamErrorState + Send> {
        Box::new(Self(Arc::clone(&self.0)))
    }
}

impl StreamErrorState for FailureCapture {
    fn observe_chunk(&mut self, _: &StreamItem) {}

    fn mark_emitted(&mut self, frame: String) -> String {
        frame
    }

    fn format_failure(&mut self, failure: &StreamFailure) -> Option<String> {
        self.0
            .lock()
            .expect("captured failures")
            .push(failure.clone());
        None
    }
}

#[rstest]
#[case::missing(None, None)]
#[case::text_code(Some(ErrorCode::Text(String::from("insufficient_quota"))), None)]
#[case::numeric_code(
    Some(ErrorCode::Number(429)),
    Some(Json::Text(String::from("provider body")))
)]
#[case::zero_code(Some(ErrorCode::Number(0)), Some(Json::Null))]
#[case::empty_code(
    Some(ErrorCode::Text(String::new())),
    Some(Json::Object(JsonObject::new()))
)]
#[case::provider_body(
    Some(ErrorCode::Number(429)),
    Some(Json::Object(object(&[
        ("code", Json::Text(String::from("insufficient_quota"))),
        ("message", Json::Text(String::from("provider quota exhausted"))),
        ("type", Json::Text(String::from("quota_error"))),
    ])))
)]
fn failure_details_reach_state(#[case] code: Option<ErrorCode>, #[case] body: Option<Json>) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let mut generator = plain_generator();
    generator.error_state = Arc::new(FailureCapture(Arc::clone(&captured)));
    let mut failure = other_failure("boom");
    failure.code = code.map(Box::new);
    failure.body = body.map(Box::new);
    failure.status_code = Some(429);
    let out = frames(body_of(
        &generator,
        source(vec![Err(failure.clone())], JsonObject::new()),
        request(&[]),
        true,
    ));
    assert_eq!(out, [DONE_FRAME]);
    assert_eq!(
        *captured.lock().expect("captured failures"),
        vec![failure.clone()]
    );
    assert_eq!(
        failure_frame(&failure),
        "data: {\"error\":{\"message\":\"boom\",\"type\":\"None\",\"param\":\"None\",\"code\":\"429\"}}\n\n"
    );
}

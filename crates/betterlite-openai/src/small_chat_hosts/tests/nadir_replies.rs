use serde_json::Value;

use crate::chat_choices::reply::TransformResponseError;
use crate::edenai_common::RESPONSE_COST_HEADER;

use super::nadir_reply;

#[test]
fn nadir_records_the_cost_after_the_parent_reply() {
    let reply = nadir_reply(br#"{"choices":[],"nadir_metadata":{"cost":{"total_cost_usd":0.25}}}"#)
        .expect("the parent transforms the reply");
    assert_eq!(
        reply.response.get("choices"),
        Some(&Value::Array(Vec::new()))
    );
    let hidden = serde_json::to_value(&reply.hidden_params).unwrap();
    assert_eq!(
        hidden["additional_headers"][RESPONSE_COST_HEADER],
        Value::from(0.25_f64)
    );
}

#[test]
fn nadir_records_no_cost_when_the_bytes_carry_none() {
    let reply = nadir_reply(br#"{"choices":[],"nadir_metadata":{"cost":{"total_cost_usd":true}}}"#)
        .expect("the parent transforms the reply");
    assert!(reply.hidden_params.is_empty());
}

#[test]
fn nadir_returns_the_parent_error_before_reading_any_cost() {
    let result = nadir_reply(b"not json");
    assert!(matches!(result, Err(TransformResponseError::Json(_))));
}

#[test]
fn nadir_returns_a_projection_failure_before_reading_any_cost() {
    let result = nadir_reply(
        br#"{"refuse_projection":true,"nadir_metadata":{"cost":{"total_cost_usd":0.25}}}"#,
    );
    assert!(matches!(result, Err(TransformResponseError::Projection(_))));
}

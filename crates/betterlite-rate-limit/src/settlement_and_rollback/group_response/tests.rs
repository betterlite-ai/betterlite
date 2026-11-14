#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;

use crate::limiter_records::{AtomicCounterMeta, ScriptReply};
use crate::settlement_and_rollback::group_response::{
    GroupFailure, GroupResponse, pipelined_group_response,
};
use crate::settlement_and_rollback::tests::support::{meta, reply};

fn one_counter() -> [AtomicCounterMeta; 1] {
    [meta("{model:gpt-4}:window", "{model:gpt-4}:requests", 1)]
}

#[test]
fn an_ok_reply_becomes_a_response_with_its_values() {
    let group = pipelined_group_response(Ok(reply(&[0, 4, 100])), &one_counter());
    let GroupResponse::Response { response, values } = group else {
        panic!("an OK reply decodes");
    };
    assert_eq!(response.overall_code, "OK");
    assert_eq!(values.len(), 3);
}

#[test]
fn an_over_limit_reply_is_reported_as_over_limit() {
    let group = pipelined_group_response(Ok(reply(&[1, 1, 9, 10])), &one_counter());
    assert!(group.over_limit().is_some());
    assert!(group.failure().is_none());
}

#[test]
fn a_batch_error_becomes_the_groups_failure() {
    let group = pipelined_group_response(Err(Arc::new(BatchError::Cancelled)), &one_counter());
    let GroupResponse::Failure(failure) = group else {
        panic!("a batch error is a failure");
    };
    assert!(matches!(failure, GroupFailure::Batch(_)));
    assert!(failure.to_string().contains("cancelled"));
}

#[test]
fn a_reply_that_is_not_an_array_is_a_reply_failure() {
    let group = pipelined_group_response(Ok(ScriptReply::Int(7)), &one_counter());
    let GroupResponse::Failure(failure) = group else {
        panic!("a non-array reply is a failure");
    };
    assert!(matches!(failure, GroupFailure::Reply(_)));
}

#[test]
fn a_reply_with_a_float_counter_is_a_decode_failure() {
    let raw = ScriptReply::Array(vec![
        ScriptReply::Int(0),
        ScriptReply::Float(1.5),
        ScriptReply::Int(100),
    ]);
    let group = pipelined_group_response(Ok(raw), &one_counter());
    let GroupResponse::Failure(failure) = group else {
        panic!("a float counter does not decode");
    };
    assert!(matches!(failure, GroupFailure::Decode(_)));
    assert!(failure.to_string().contains("not an integer"));
}

#[test]
fn a_reply_without_every_counter_is_a_decode_failure() {
    let group = pipelined_group_response(Ok(reply(&[0, 1])), &one_counter());
    let GroupResponse::Failure(failure) = group else {
        panic!("a short reply does not decode");
    };
    assert!(matches!(failure, GroupFailure::Decode(_)));
}

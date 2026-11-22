#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::api::video_list_call::VideoListReply;
use crate::api::video_remix_call::error::VideoCallError;
use crate::api::video_remix_call::support::{Bench, Case, LIST_BODY, Plan, json, list_call};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn list_mock_response_json_string_returns_objects() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.mock_response = Some(json(
        r#"[{"id":"video_1","object":"video","status":"queued"},{"id":"video_2","object":"video","status":"completed"}]"#,
    ));
    let reply = bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect("the mock answers");
    let VideoListReply::Objects(objects) = reply else {
        panic!("a mock list is objects");
    };
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[1].id, "video_2");
    assert_eq!(bench.fixture.state.call_type, "");
    assert!(bench.requests().is_empty());
}

#[test]
fn list_mock_response_invalid_json_is_a_value_error() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.mock_response = Some(Json::Text("not json".to_owned()));
    let failure = bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect_err("the mock text is JSON");
    assert!(matches!(failure.error, VideoCallError::Mock(_)));
    assert_eq!(failure.class.metadata().status_code, 500);
    assert!(bench.requests().is_empty());
}

#[test]
fn list_mock_response_null_is_no_mock_response() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.mock_response = Some(Json::Null);
    let reply = bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect("the provider answers");
    assert!(matches!(reply, VideoListReply::Page(_)));
    assert_eq!(bench.requests().len(), 1);
}

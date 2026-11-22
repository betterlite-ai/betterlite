use betterlite_models::decisions::Json;

use crate::api::video_list_call::VideoListCall;
use crate::api::video_remix_call::support::{
    Bench, LIST_BODY, encoded_id, headers, json, params, request_header, request_query, run_list,
};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn list_pagination_is_sent_as_query() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        after: Some(encoded_id("video_7", "openai")),
        limit: Some(5),
        order: Some("desc".to_owned()),
        extra_query: Some(params(r#"{"extra":"yes","order":"asc"}"#)),
        ..VideoListCall::default()
    };
    run_list(&mut bench, &call, REQUEST_TIMEOUT).expect("the list answers");
    let requests = bench.requests();
    let request = &requests[0];
    assert_eq!(request_query(request, "after").as_deref(), Some("video_7"));
    assert_eq!(request_query(request, "limit").as_deref(), Some("5"));
    assert_eq!(request_query(request, "order").as_deref(), Some("asc"));
    assert_eq!(request_query(request, "extra").as_deref(), Some("yes"));
}

#[test]
fn list_params_are_logged_as_given() {
    let mut bench = Bench::new(200, LIST_BODY);
    let raw_after = encoded_id("video_7", "openai");
    let call = VideoListCall {
        after: Some(raw_after.clone()),
        limit: Some(5),
        order: Some("desc".to_owned()),
        ..VideoListCall::default()
    };
    run_list(&mut bench, &call, REQUEST_TIMEOUT).expect("the list answers");
    let details = &bench.fixture.state.model_call_details;
    assert_eq!(details.get("after"), Some(&Json::Text(raw_after.clone())));
    assert_eq!(details.get("limit"), Some(&json("5")));
    assert_eq!(details.get("order"), Some(&Json::Text("desc".to_owned())));
    let Some(Json::Object(litellm_params)) = details.get("litellm_params") else {
        panic!("litellm_params is an object");
    };
    assert_eq!(litellm_params.get("after"), Some(&Json::Text(raw_after)));
    assert_eq!(litellm_params.get("limit"), Some(&json("5")));
}

#[test]
fn list_extra_headers_reach_the_request() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        extra_headers: Some(headers(&[("x-test", "1")])),
        ..VideoListCall::default()
    };
    run_list(&mut bench, &call, REQUEST_TIMEOUT).expect("the list answers");
    let requests = bench.requests();
    assert_eq!(request_header(&requests[0], "x-test"), "1");
}

#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;

use crate::api::video_remix_call::support::{
    Bench, Case, LIST_BODY, Plan, headers, json, list_call, params, request_header, request_query,
};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn list_pagination_is_sent_as_query() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.after = Some(encode_video_id_with_provider("video_7", "openai", None));
    call.limit = Some(5);
    call.order = Some("desc".to_owned());
    call.extra_query = Some(params(r#"{"extra":"yes","order":"asc"}"#));
    bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect("the list answers");
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
    let raw_after = encode_video_id_with_provider("video_7", "openai", None);
    let mut call = list_call();
    call.after = Some(raw_after.clone());
    call.limit = Some(5);
    call.order = Some("desc".to_owned());
    bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect("the list answers");
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
    let mut call = list_call();
    call.extra_headers = Some(headers(&[("x-test", "1")]));
    bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect("the list answers");
    let requests = bench.requests();
    assert_eq!(request_header(&requests[0], "x-test"), "1");
}

#![cfg(test)]

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;

use super::support::{
    Bench, Case, LIST_BODY, Plan, VIDEO_BODY, json, list_call, params, remix_call,
};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn remix_extra_body_reaches_the_request_body() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.video_id = "video_5".to_owned();
    call.prompt = "make it blue".to_owned();
    call.custom_llm_provider = Some("openai".to_owned());
    call.extra_body = Some(params(r#"{"seconds": "8"}"#));
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the remix answers");
    let requests = bench.requests();
    let RequestBody::Json(bytes) = &requests[0].body else {
        panic!("the remix request carries a JSON body");
    };
    let body: Json = serde_json::from_slice(bytes).expect("the body is JSON");
    let Json::Object(fields) = body else {
        panic!("the remix body is an object");
    };
    assert_eq!(fields.get("prompt"), Some(&json(r#""make it blue""#)));
    assert_eq!(fields.get("seconds"), Some(&json(r#""8""#)));
}

#[test]
fn remix_extra_query_and_list_extra_body_are_dropped() {
    let mut remix_bench = Bench::new(200, VIDEO_BODY);
    let mut remix_input = remix_call();
    remix_input.video_id = "video_5".to_owned();
    remix_input.prompt = "make it blue".to_owned();
    remix_input.custom_llm_provider = Some("openai".to_owned());
    remix_input.extra_query = Some(params(r#"{"extra": "yes"}"#));
    remix_bench
        .video_answer(Plan::seconds(Case::Remix(&remix_input), REQUEST_TIMEOUT))
        .expect("the remix answers");
    let remix_requests = remix_bench.requests();
    assert!(remix_requests[0].query.is_empty());

    let mut list_bench = Bench::new(200, LIST_BODY);
    let mut list_input = list_call();
    list_input.extra_body = Some(params(r#"{"extra": "yes"}"#));
    list_bench
        .list_answer(Plan::seconds(Case::List(&list_input), REQUEST_TIMEOUT))
        .expect("the list answers");
    let list_requests = list_bench.requests();
    assert!(matches!(&list_requests[0].body, RequestBody::Empty));
}

#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::api::video_list_call::VideoListCall;
use crate::api::video_remix_call::support::{Bench, Case, LIST_BODY, Plan};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn list_call_type_is_video_list() {
    let mut bench = Bench::new(200, LIST_BODY);
    let call = VideoListCall {
        custom_llm_provider: Some("openai".to_owned()),
        user: Some("user-1".to_owned()),
        ..VideoListCall::default()
    };
    bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect("the list answers");
    let state = &bench.fixture.state;
    assert_eq!(state.call_type, "video_list");
    let details = &state.model_call_details;
    assert_eq!(details.get("model"), Some(&Json::Text(String::new())));
    assert_eq!(details.get("call_type"), Some(&Json::Text(String::new())));
    assert_eq!(details.get("user"), Some(&Json::Text("user-1".to_owned())));
    assert_eq!(
        details.get("custom_llm_provider"),
        Some(&Json::Text("openai".to_owned()))
    );
    assert_eq!(details.get("after"), Some(&Json::Null));
    assert_eq!(details.get("limit"), Some(&Json::Null));
    assert_eq!(details.get("order"), Some(&Json::Null));
    let Some(Json::Object(optional)) = details.get("optional_params") else {
        panic!("optional_params is an object");
    };
    assert_eq!(optional.get("after"), Some(&Json::Null));
    let Some(Json::Object(litellm_params)) = details.get("litellm_params") else {
        panic!("litellm_params is an object");
    };
    assert_eq!(litellm_params.get("limit"), Some(&Json::Null));
}

#[test]
fn list_call_type_is_set_before_the_handler() {
    let mut bench = Bench::new(500, r#"{"error":{"message":"upstream failed"}}"#);
    bench
        .list_answer(Plan::seconds(
            Case::List(&VideoListCall::default()),
            REQUEST_TIMEOUT,
        ))
        .expect_err("the provider answer is a failure");
    assert_eq!(bench.fixture.state.call_type, "video_list");
    assert_eq!(bench.requests().len(), 1);
}

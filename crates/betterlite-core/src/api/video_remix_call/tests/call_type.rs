#![cfg(test)]

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{Bench, Case, Plan, VIDEO_BODY, json, remix_call};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn remix_call_type_is_video_remix() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.video_id = "video_5".to_owned();
    call.prompt = "make it blue".to_owned();
    call.custom_llm_provider = Some("openai".to_owned());
    call.user = Some("user-1".to_owned());
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the remix answers");
    let state = &bench.fixture.state;
    assert_eq!(state.call_type, "video_remix");
    let details = &state.model_call_details;
    assert_eq!(details.get("model"), Some(&Json::Text(String::new())));
    assert_eq!(details.get("call_type"), Some(&Json::Text(String::new())));
    assert_eq!(details.get("user"), Some(&Json::Text("user-1".to_owned())));
    assert_eq!(
        details.get("custom_llm_provider"),
        Some(&Json::Text("openai".to_owned()))
    );
    assert_eq!(
        details.get("video_id"),
        Some(&Json::Text("video_5".to_owned()))
    );
    assert_eq!(
        details.get("prompt"),
        Some(&Json::Text("make it blue".to_owned()))
    );
}

#[test]
fn remix_call_type_logs_the_call_params() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.video_id = "video_5".to_owned();
    call.prompt = "make it blue".to_owned();
    call.custom_llm_provider = Some("openai".to_owned());
    call.litellm_call_id = Some("call-9".to_owned());
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the remix answers");
    let details = &bench.fixture.state.model_call_details;
    let Some(Json::Object(optional)) = details.get("optional_params") else {
        panic!("optional_params is an object");
    };
    assert_eq!(
        optional.get("video_id"),
        Some(&Json::Text("video_5".to_owned()))
    );
    let Some(Json::Object(params)) = details.get("litellm_params") else {
        panic!("litellm_params is an object");
    };
    assert_eq!(
        params.get("litellm_call_id"),
        Some(&Json::Text("call-9".to_owned()))
    );
    assert_eq!(
        params.get("prompt"),
        Some(&Json::Text("make it blue".to_owned()))
    );
}

#[test]
fn remix_call_type_is_set_before_the_handler() {
    let mut bench = Bench::new(500, r#"{"error":{"message":"upstream failed"}}"#);
    let call = remix_call();
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("the provider answer is a failure");
    assert_eq!(failure.class.metadata().status_code, 500);
    assert_eq!(bench.fixture.state.call_type, "video_remix");
    assert_eq!(bench.requests().len(), 1);
}

#[test]
fn remix_metadata_reaches_the_logging_params() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut metadata = IndexMap::new();
    metadata.insert("user_id".to_owned(), JsonValue::Text("u-1".to_owned()));
    let mut call = remix_call();
    call.litellm_metadata = Some(metadata);
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the remix answers");
    let details = &bench.fixture.state.model_call_details;
    let Some(Json::Object(params)) = details.get("litellm_params") else {
        panic!("litellm_params is an object");
    };
    assert_eq!(
        params.get("litellm_metadata"),
        Some(&json(r#"{"user_id":"u-1"}"#))
    );
    assert_eq!(params.get("metadata"), Some(&json(r#"{"user_id":"u-1"}"#)));
}

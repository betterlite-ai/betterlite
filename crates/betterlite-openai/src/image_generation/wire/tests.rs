#![cfg(test)]

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use http::header::AUTHORIZATION;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::outbound_request;
use crate::image_generation::call::{ImageGenerationCall, prepare_image_call};
use crate::image_generation::decode::original_response;
use crate::image_generation::error::{ErrorStatus, ImageGenerationFailure, ImageGenerationSource};
use crate::image_generation::logging::{ImageGenerationLogger, PostCallRecord, PreCallRecord};
use crate::image_generation::request::{
    ImageGenerationOutcome, begin_image_generation, finish_image_generation, send_image_generation,
};
use crate::image_generation::tests::{
    FakeTransport, body_of, call, header_text, ok_transport, options,
};

#[derive(Default)]
struct Spy {
    pre_calls: Vec<String>,
    posts: Vec<PostCallRecord>,
}

impl ImageGenerationLogger for Spy {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre_calls.push(record.input);
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.posts.push(record);
    }
}

async fn spied(
    call: ImageGenerationCall,
    transport: &FakeTransport,
) -> (Result<ImageGenerationOutcome, ImageGenerationFailure>, Spy) {
    let mut spy = Spy::default();
    let outcome = match begin_image_generation(call, &mut spy) {
        Ok(prepared) => {
            let sent = send_image_generation(&prepared, transport).await;
            finish_image_generation(&prepared, sent, &mut spy)
        }
        Err(failure) => Err(failure),
    };
    (outcome, spy)
}

#[tokio::test]
async fn extra_query_and_extra_body_leave_the_json_body() {
    let transport = ok_transport();
    let params = options(
        r#"{"extra_query":{"api-version":"v2","n":3,"on":true},"extra_body":{"seed":9,"model":"over"}}"#,
    );
    let (outcome, _) = spied(call("dall-e-2", params), &transport).await;
    outcome.unwrap();
    let requests = transport.requests();
    assert_eq!(
        requests[0].query,
        vec![
            (String::from("api-version"), String::from("v2")),
            (String::from("n"), String::from("3")),
            (String::from("on"), String::from("true")),
        ]
    );
    let Json::Object(body) = &requests[0].body else {
        panic!("the request body is an object");
    };
    assert!(!body.contains_key("extra_query"));
    assert!(!body.contains_key("extra_body"));
    assert_eq!(body.get("seed"), Some(&Json::Number(Number::from(9_i64))));
    assert_eq!(body.get("model"), Some(&Json::Text(String::from("over"))));
}

#[rstest]
#[case::list_headers(r#"{"extra_headers":[1]}"#, "extra_headers")]
#[case::number_header_value(r#"{"extra_headers":{"x":1}}"#, "extra_headers")]
#[case::text_query(r#"{"extra_query":"a=b"}"#, "extra_query")]
#[case::list_query_value(r#"{"extra_query":{"a":[1]}}"#, "extra_query")]
#[case::text_body(r#"{"extra_body":"x"}"#, "extra_body")]
fn extras_of_the_wrong_type_name_their_field(#[case] params: &str, #[case] field: &str) {
    let prepared = prepare_image_call(call("dall-e-2", options(params))).unwrap();
    let failure = outbound_request(&prepared).unwrap_err();
    assert!(
        matches!(&failure, ImageGenerationSource::Extra { field: found } if *found == field),
        "{failure:?}"
    );
    assert_eq!(failure.status(), ErrorStatus::Missing);
}

#[test]
fn null_extras_are_left_out() {
    let params = options(r#"{"extra_headers":null,"extra_query":null,"extra_body":null}"#);
    let prepared = prepare_image_call(call("dall-e-2", params)).unwrap();
    let request = outbound_request(&prepared).unwrap();
    assert!(request.query.is_empty());
    let body = body_of(&request);
    assert!(!body.contains_key("extra_headers"));
    assert!(!body.contains_key("extra_query"));
    assert!(!body.contains_key("extra_body"));
}

#[rstest]
#[case::header_name(r#"{"extra_headers":{"bad name":"x"}}"#)]
#[case::header_value(r#"{"extra_headers":{"x-ok":"line\nbreak"}}"#)]
fn invalid_extra_headers_are_typed_errors(#[case] params: &str) {
    let prepared = prepare_image_call(call("dall-e-2", options(params))).unwrap();
    let failure = outbound_request(&prepared).unwrap_err();
    assert!(
        matches!(
            failure,
            ImageGenerationSource::RequestHeaderName(_)
                | ImageGenerationSource::RequestHeader { .. }
        ),
        "{failure:?}"
    );
}

#[test]
fn organization_is_sent_only_when_set() {
    let mut without = call("dall-e-2", options("{}"));
    without.organization = None;
    let request = outbound_request(&prepare_image_call(without).unwrap()).unwrap();
    assert_eq!(header_text(&request.headers, "openai-organization"), None);
    assert_eq!(
        header_text(&request.headers, "content-type"),
        Some("application/json")
    );
}

#[test]
fn the_authorization_header_is_sensitive() {
    let prepared = prepare_image_call(call("dall-e-2", options("{}"))).unwrap();
    let request = outbound_request(&prepared).unwrap();
    assert!(request.headers.get(AUTHORIZATION).unwrap().is_sensitive());
    assert!(!format!("{:?}", request.headers).contains("sk-test"));
    assert!(
        !request
            .headers
            .get("openai-organization")
            .unwrap()
            .is_sensitive()
    );
}

#[rstest]
#[case::sync(false)]
#[case::async_mode(true)]
#[tokio::test]
async fn a_missing_api_key_never_reaches_the_transport(#[case] delegate_async: bool) {
    let transport = ok_transport();
    let mut keyless = call("dall-e-2", options("{}"));
    keyless.api_key = None;
    keyless.delegate_async = delegate_async;
    let (outcome, spy) = spied(keyless, &transport).await;
    let failure = outcome.unwrap_err();
    assert!(transport.requests().is_empty());
    assert!(spy.pre_calls.is_empty());
    assert_eq!(spy.posts.len(), 1);
    assert_eq!(failure.mapped(), !delegate_async);
    assert_eq!(failure.status(), (!delegate_async).then_some(500));
    assert!(matches!(
        failure.failure_source(),
        ImageGenerationSource::MissingApiKey
    ));
    assert_eq!(spy.posts[0].data.is_some(), !delegate_async);
}

#[rstest]
#[case::sync(false)]
#[case::async_mode(true)]
#[tokio::test]
async fn a_key_that_is_no_header_value_fails_after_pre_call(#[case] delegate_async: bool) {
    let transport = ok_transport();
    let mut bad = call("dall-e-2", options("{}"));
    bad.api_key = Some(SecretString::from("sk\nbad"));
    bad.delegate_async = delegate_async;
    let (outcome, spy) = spied(bad, &transport).await;
    let failure = outcome.unwrap_err();
    assert!(transport.requests().is_empty());
    assert_eq!(spy.pre_calls.len(), 1);
    assert_eq!(spy.posts.len(), 1);
    assert_eq!(failure.mapped(), !delegate_async);
    assert_eq!(failure.status(), (!delegate_async).then_some(500));
    assert_eq!(spy.posts[0].data.is_some(), !delegate_async);
    assert!(matches!(
        failure.failure_source(),
        ImageGenerationSource::RequestHeader { .. }
    ));
}

#[tokio::test]
async fn post_call_receives_the_provider_body_with_absent_fields_null() {
    let transport = ok_transport();
    let (outcome, spy) = spied(call("dall-e-2", options("{}")), &transport).await;
    outcome.unwrap();
    let Json::Object(original) = &spy.posts[0].original_response else {
        panic!("post_call receives the parsed provider body");
    };
    assert_eq!(
        original.get("created"),
        Some(&Json::Number(Number::from(7_i64)))
    );
    assert_eq!(original.get("usage"), Some(&Json::Null));
    assert_eq!(original.get("size"), Some(&Json::Null));
    let Some(Json::List(items)) = original.get("data") else {
        panic!("the data list is kept");
    };
    let Json::Object(image) = &items[0] else {
        panic!("a data item is an object");
    };
    assert_eq!(image.get("b64_json"), Some(&Json::Null));
    assert_eq!(image.get("revised_prompt"), Some(&Json::Null));
    assert_eq!(
        image.get("url"),
        Some(&Json::Text(String::from("https://example.test/a.png")))
    );
}

#[test]
fn original_response_fills_usage_details() {
    let found = original_response(br#"{"usage":{"input_tokens":1},"extra":5}"#).unwrap();
    let Json::Object(fields) = found else {
        panic!("the dump is an object");
    };
    assert_eq!(
        fields.get("extra"),
        Some(&Json::Number(Number::from(5_i64)))
    );
    assert_eq!(fields.get("data"), Some(&Json::Null));
    let Some(Json::Object(usage)) = fields.get("usage") else {
        panic!("the usage object is kept");
    };
    assert_eq!(usage.get("output_tokens_details"), Some(&Json::Null));
    assert_eq!(
        usage.get("input_tokens"),
        Some(&Json::Number(Number::from(1_i64)))
    );
}

#[rstest]
#[case::sync(false)]
#[case::async_mode(true)]
#[tokio::test]
async fn a_noninteger_max_retries_is_wrapped_before_dispatch(#[case] delegate_async: bool) {
    let transport = ok_transport();
    let mut bad = call("dall-e-2", options(r#"{"max_retries":"3"}"#));
    bad.delegate_async = delegate_async;
    let (outcome, spy) = spied(bad, &transport).await;
    let failure = outcome.unwrap_err();
    assert!(failure.mapped());
    assert_eq!(failure.status(), Some(422));
    assert!(transport.requests().is_empty());
    assert!(spy.pre_calls.is_empty());
}

#[rstest]
#[case::with_usage(
    r#"{"created":7,"data":[],"usage":{"input_tokens":4,"input_tokens_details":{"image_tokens":1,"text_tokens":3},"output_tokens":8,"total_tokens":12}}"#,
    true
)]
#[case::without_usage(r#"{"created":7,"data":[]}"#, false)]
#[tokio::test]
async fn the_request_path_reply_carries_the_sdk_output_tokens_details(
    #[case] body: &str,
    #[case] has_usage: bool,
) {
    let transport = FakeTransport::answering(200, &[], body);
    let (outcome, _) = spied(call("gpt-image-1", options("{}")), &transport).await;
    let usage = serde_json::to_value(&outcome.unwrap().reply.usage).unwrap();
    assert_eq!(
        usage.get("output_tokens_details"),
        has_usage.then_some(&serde_json::Value::Null)
    );
}

use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::evals::error::EvalsError;

use super::support::{
    Operation, REQUEST_TIMEOUT, RealConfigs, Rig, assert_no_request, call, json_server, kwargs,
    openai_kwargs, run, text, url,
};
use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_lifecycle::error::EvalLifecycleError;

#[tokio::test]
async fn delete_eval_reports_a_rejected_key_as_authentication() {
    let body = r#"{"error":{"message":"Incorrect API key provided: sk-test.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#;
    let serving = json_server("401 Unauthorized", body, Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("a 401 ends the call");
    serving.finish().expect("the fixture finishes");
    let ErrorClass::Authentication(details) = &failure.class else {
        panic!("an authentication error, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 401);
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert!(details.message.contains("Incorrect API key provided"));
}

#[tokio::test]
async fn delete_eval_reports_a_missing_eval_as_not_found() {
    let body = r#"{"error":{"message":"no such eval"}}"#;
    let serving = json_server("404 Not Found", body, Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("a 404 ends the call");
    serving.finish().expect("the fixture finishes");
    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("a not found error, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 404);
    assert!(details.message.contains("no such eval"));
}

#[tokio::test]
async fn cancel_eval_reports_a_missing_eval_as_not_found() {
    let body = r#"{"error":{"message":"no such eval"}}"#;
    let serving = json_server("404 Not Found", body, Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let failure = run(&mut rig, Operation::Cancel, &request)
        .await
        .expect_err("a 404 ends the call");
    serving.finish().expect("the fixture finishes");
    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("a not found error, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 404);
}

#[tokio::test]
async fn delete_eval_fails_on_a_reply_that_is_not_a_delete_response() {
    let serving = json_server("200 OK", "{}", Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("an empty object is not a delete response");
    serving.finish().expect("the fixture finishes");
    let EvalLifecycleError::Handler(EvalHttpError::Api(EvalsError::ReplyModel { .. })) =
        &failure.error
    else {
        panic!("a reply model failure, got {}", failure.error);
    };
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("an APIConnectionError, got {:?}", failure.class);
    };
    assert!(
        context
            .message
            .contains("the eval reply does not match its model")
    );
}

#[tokio::test]
async fn cancel_eval_fails_on_a_reply_that_is_not_a_cancel_response() {
    let serving = json_server("200 OK", "{}", Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let failure = run(&mut rig, Operation::Cancel, &request)
        .await
        .expect_err("an empty object is not a cancel response");
    serving.finish().expect("the fixture finishes");
    let EvalLifecycleError::Handler(EvalHttpError::Api(EvalsError::ReplyModel { .. })) =
        &failure.error
    else {
        panic!("a reply model failure, got {}", failure.error);
    };
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("an APIConnectionError, got {:?}", failure.class);
    };
    assert!(
        context
            .message
            .contains("the eval reply does not match its model")
    );
}

#[tokio::test]
async fn delete_eval_requires_the_openai_key_before_the_send() {
    let serving = json_server("200 OK", "{}", Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = kwargs(&[("api_base", text("https://api.example"))]);
    let request = call("eval_1", &params);
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("no key, no call");
    let EvalLifecycleError::Config(EvalsError::ApiKeyRequired) = &failure.error else {
        panic!("an api key error, got {}", failure.error);
    };
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("an APIConnectionError, got {:?}", failure.class);
    };
    assert!(
        context
            .message
            .contains("OPENAI_API_KEY is required for Evals API")
    );
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_no_request(serving);
}

#[tokio::test]
async fn delete_eval_fails_on_a_pre_call_that_stops_the_call() {
    let serving = json_server("200 OK", "{}", Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    rig.fixture.state.request_logging.raw_request_only = true;
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("the captured raw request stops the call");
    assert_no_request(serving);
    let EvalLifecycleError::Handler(EvalHttpError::Logging(PrePostError::RawRequestCaptured)) =
        &failure.error
    else {
        panic!("a pre-call failure, got {}", failure.error);
    };
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("an APIConnectionError, got {:?}", failure.class);
    };
    assert!(
        context
            .message
            .contains("the raw request was captured instead of sent")
    );
}

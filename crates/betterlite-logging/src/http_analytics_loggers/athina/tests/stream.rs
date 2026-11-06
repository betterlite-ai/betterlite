use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::http_analytics_loggers::athina::{AthinaLogger, AthinaSettings};
use crate::http_analytics_loggers::tests::support::{COMPLETION, Capture, event, field, json};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01Z";
const STREAM_KWARGS: &str = r#"{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"Hello"}],"litellm_params":{"metadata":{}}}"#;
const PLAIN_KWARGS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"litellm_params":{"metadata":{}}}"#;

fn buffered_kwargs() -> Json {
    let mut kwargs = json(STREAM_KWARGS);
    let Json::Object(fields) = &mut kwargs else {
        panic!("the fixture kwargs are an object");
    };
    fields.insert("complete_streaming_response".to_owned(), json(COMPLETION));
    kwargs
}

#[tokio::test]
async fn athina_skips_a_stream_without_the_buffered_response() {
    let capture = Capture::default();
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&BTreeMap::new()),
    };
    logger
        .log_event(&event(&json(STREAM_KWARGS), None, START, END))
        .await;
    assert!(capture.requests().is_empty());
}

#[tokio::test]
async fn athina_logs_the_buffered_response_in_streaming_mode() {
    let capture = Capture::default();
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&BTreeMap::new()),
    };
    logger
        .log_event(&event(&buffered_kwargs(), None, START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    let body = &requests[0].body;
    assert_eq!(field(body, "response"), &json(COMPLETION));
    assert_eq!(
        field(field(body, "request"), "complete_streaming_response"),
        &json(COMPLETION)
    );
}

#[tokio::test]
async fn athina_logs_the_response_object_in_non_streaming_mode() {
    let capture = Capture::default();
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&BTreeMap::new()),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(PLAIN_KWARGS), Some(&response), START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(field(&requests[0].body, "response"), &response);
    assert_eq!(field(&requests[0].body, "response_time"), &json("1000"));
}

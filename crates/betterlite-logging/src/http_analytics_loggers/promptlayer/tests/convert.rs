use crate::http_analytics_loggers::promptlayer::{PromptLayerLogger, PromptLayerSettings};
use crate::http_analytics_loggers::tests::support::{
    COMPLETION, Capture, environment, event, field, json,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01Z";
const ENVIRONMENT: &[(&str, &str)] = &[("PROMPTLAYER_API_KEY", "pl-key")];
const KWARGS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"optional_params":{"temperature":0.2,"top_p":1},"litellm_params":{"metadata":{"pl_tags":["prod","chat"],"user":"u1","team":"t1"}}}"#;

#[tokio::test]
async fn promptlayer_sends_the_response_dump_and_extracts_the_tags() {
    let capture = Capture::default();
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    let body = &requests[0].body;
    assert_eq!(field(body, "request_response"), &response);
    assert_eq!(field(body, "tags"), &json(r#"["prod","chat"]"#));
}

#[tokio::test]
async fn promptlayer_skips_a_call_without_a_response_object() {
    let capture = Capture::default();
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    logger
        .log_event(&event(&json(KWARGS), None, START, END))
        .await;
    assert!(capture.requests().is_empty());
}

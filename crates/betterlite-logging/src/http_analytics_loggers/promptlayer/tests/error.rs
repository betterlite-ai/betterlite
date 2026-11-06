use crate::http_analytics_loggers::promptlayer::{PromptLayerLogger, PromptLayerSettings};
use crate::http_analytics_loggers::tests::support::{
    Answer, COMPLETION, Capture, Captured, environment, event, json,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01Z";
const ENVIRONMENT: &[(&str, &str)] = &[("PROMPTLAYER_API_KEY", "pl-key")];
const KWARGS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"optional_params":{},"litellm_params":{"metadata":{"user":"u1"}}}"#;

#[tokio::test]
async fn promptlayer_swallows_a_delivery_failure_and_logs_it() {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::with_answers([Answer::Fail]);
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert_eq!(capture.requests().len(), 1);
    assert!(captured.text().contains("PromptLayer logger error"));
}

#[tokio::test]
async fn promptlayer_swallows_a_server_status_and_logs_it() {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::with_answers([Answer::Status(500)]);
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert_eq!(capture.requests().len(), 1);
    assert!(captured.text().contains("PromptLayer logger error"));
}

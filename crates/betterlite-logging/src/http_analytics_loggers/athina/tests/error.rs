use std::collections::BTreeMap;

use crate::http_analytics_loggers::athina::{AthinaLogger, AthinaSettings};
use crate::http_analytics_loggers::tests::support::{
    Answer, COMPLETION, Capture, Captured, environment, event, json,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01Z";
const KEY: &[(&str, &str)] = &[("ATHINA_API_KEY", "athina-key-1")];
const KWARGS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"litellm_params":{"metadata":{}}}"#;

#[tokio::test]
async fn athina_swallows_a_delivery_failure_and_logs_it() {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::with_answers([Answer::Fail]);
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&environment(KEY)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert_eq!(capture.requests().len(), 1);
    assert!(captured.text().contains("Athina logger error"));
}

#[tokio::test]
async fn athina_swallows_a_missing_api_key() {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::default();
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&BTreeMap::new()),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert!(capture.requests().is_empty());
    assert!(captured.text().contains("ATHINA_API_KEY"));
}

use super::support::{BASE, Scenario, answer, failing, json, logger};
use crate::braintrust_logging::error::BraintrustError;
use crate::braintrust_logging::event::CallbackPath;

#[tokio::test]
async fn insert_posts_events_to_the_project_logs_endpoint() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests[0].url, format!("{BASE}/project_logs/p/insert"));
    assert_eq!(
        requests[0].body,
        json(
            r#"{"events":[{"id":null,"input":null,"metadata":{"project_id":"p"},"span_attributes":{"name":"Chat Completion","type":"llm"},"tags":[],"output":[]}]}"#
        )
    );
}

#[tokio::test]
async fn insert_failure_returns_the_response_text() {
    let (logger, _replay) = logger(vec![failing(500, r#"{"error":"insert failed"}"#)], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    let failure = logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap_err();
    let BraintrustError::Status { status, body } = failure else {
        panic!("expected a status failure");
    };
    assert_eq!(status, 500);
    assert_eq!(body, r#"{"error":"insert failed"}"#);
}

#[tokio::test]
async fn failure_events_send_nothing() {
    let (logger, replay) = logger(Vec::new(), &[]);
    logger.log_failure_event();
    assert!(replay.requests().is_empty());
}

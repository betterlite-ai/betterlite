use betterlite_models::decisions::Json;

use super::support::{
    ResponseKind, Scenario, answer, at, at_path, json, logger, model_completion, posted,
};
use crate::braintrust_logging::payload::CallbackPath;

fn with_usage() -> Scenario {
    let mut scenario = Scenario::empty();
    scenario.response_kind = ResponseKind::Model;
    scenario.response_body = model_completion();
    scenario.response_cost = json("0.0012");
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    scenario
}

#[tokio::test]
async fn sync_time_to_first_token_is_the_call_duration() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    logger
        .log_success_event(&with_usage().event(), CallbackPath::Sync)
        .await
        .unwrap();
    let metrics = posted(&replay, 0, &["events", "0", "metrics"]);
    assert_eq!(
        metrics,
        json(
            r#"{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8,"total_cost":0.0012,"time_to_first_token":1.5,"start":1700000000.0,"end":1700000001.5}"#
        )
    );
}

#[tokio::test]
async fn async_time_to_first_token_is_the_completion_start_delta() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = with_usage();
    scenario.api_call_start_time = Some(at(1_700_000_000, 0));
    scenario.completion_start_time = Some(at(1_700_000_000, 750_000_000));
    logger
        .log_success_event(&scenario.event(), CallbackPath::Async)
        .await
        .unwrap();
    let metrics = posted(&replay, 0, &["events", "0", "metrics"]);
    assert_eq!(at_path(&metrics, &["time_to_first_token"]), &json("0.75"));
    assert_eq!(at_path(&metrics, &["start"]), &json("1700000000.0"));
    assert_eq!(at_path(&metrics, &["end"]), &json("1700000001.5"));
}

#[tokio::test]
async fn async_time_to_first_token_needs_both_times() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = with_usage();
    scenario.api_call_start_time = Some(at(1_700_000_000, 0));
    logger
        .log_success_event(&scenario.event(), CallbackPath::Async)
        .await
        .unwrap();
    let metrics = posted(&replay, 0, &["events", "0", "metrics"]);
    assert!(matches!(metrics, Json::Object(fields) if !fields.contains_key("time_to_first_token")));
}

use betterlite_models::decisions::Json;

use super::support::{
    ResponseKind, Scenario, answer, at_path, json, logger, model_completion, posted, posted_event,
};
use crate::braintrust_logging::event::CallbackPath;

fn scenario(kind: ResponseKind, body: Json) -> Scenario {
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    scenario.response_kind = kind;
    scenario.response_body = body;
    scenario
}

#[tokio::test]
async fn metrics_carry_tokens_cost_and_timestamps() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut event = scenario(ResponseKind::Model, model_completion());
    event.response_cost = json("0.0012");
    logger
        .log_success_event(&event.event(), CallbackPath::Sync)
        .await
        .unwrap();
    assert_eq!(
        posted(&replay, 0, &["events", "0", "metrics"]),
        json(
            r#"{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8,"total_cost":0.0012,"time_to_first_token":1.5,"start":1700000000.0,"end":1700000001.5}"#
        )
    );
}

#[tokio::test]
async fn metrics_are_absent_without_usage() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let response = json(
        r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#,
    );
    logger
        .log_success_event(
            &scenario(ResponseKind::Model, response).event(),
            CallbackPath::Sync,
        )
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert!(matches!(event, Json::Object(fields) if !fields.contains_key("metrics")));
}

#[tokio::test]
async fn text_completion_metrics_default_to_zero() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let response = json(
        r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"m","choices":[{"text":"hello","index":0,"finish_reason":"length"}]}"#,
    );
    logger
        .log_success_event(
            &scenario(ResponseKind::TextCompletion, response).event(),
            CallbackPath::Sync,
        )
        .await
        .unwrap();
    let metrics = posted(&replay, 0, &["events", "0", "metrics"]);
    assert_eq!(at_path(&metrics, &["prompt_tokens"]), &json("0"));
    assert_eq!(at_path(&metrics, &["completion_tokens"]), &json("0"));
    assert_eq!(at_path(&metrics, &["total_tokens"]), &json("0"));
    assert_eq!(at_path(&metrics, &["total_cost"]), &Json::Null);
}

#[tokio::test]
async fn embedding_metrics_default_to_zero() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let response = json(r#"{"object":"list","data":[],"model":"m"}"#);
    logger
        .log_success_event(
            &scenario(ResponseKind::Embedding, response).event(),
            CallbackPath::Sync,
        )
        .await
        .unwrap();
    let metrics = posted(&replay, 0, &["events", "0", "metrics"]);
    assert_eq!(at_path(&metrics, &["prompt_tokens"]), &json("0"));
    assert_eq!(at_path(&metrics, &["total_tokens"]), &json("0"));
}

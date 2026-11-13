use betterlite_models::decisions::Json;

use super::support::{ResponseKind, Scenario, answer, json, logger, posted, posted_event};
use crate::braintrust_logging::payload::CallbackPath;

#[tokio::test]
async fn model_choices_are_logged_as_choice_dicts() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    scenario.response_kind = ResponseKind::Model;
    scenario.response_body = json(
        r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"claude","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"end_turn"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}"#,
    );
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    assert_eq!(
        posted(&replay, 0, &["events", "0", "output"]),
        json(
            r#"[{"finish_reason":"stop","index":0,"message":{"content":null,"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}],"function_call":null,"provider_specific_fields":null},"provider_specific_fields":{"native_finish_reason":"end_turn"}}]"#
        )
    );
}

#[tokio::test]
async fn embedding_responses_log_an_empty_output() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    scenario.response_kind = ResponseKind::Embedding;
    scenario.response_body = json(
        r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":4,"total_tokens":4}}"#,
    );
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    assert_eq!(posted(&replay, 0, &["events", "0", "output"]), json("[]"));
}

#[tokio::test]
async fn call_type_embedding_logs_an_empty_output() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    scenario.call_type = Some("embedding".to_owned());
    scenario.response_kind = ResponseKind::Model;
    scenario.response_body = json(
        r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}"#,
    );
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    assert_eq!(posted(&replay, 0, &["events", "0", "output"]), json("[]"));
}

#[tokio::test]
async fn image_responses_log_an_empty_output() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    scenario.response_kind = ResponseKind::Image;
    scenario.response_body =
        json(r#"{"created":1700000000,"data":[{"url":"https://example.com/a.png"}]}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    assert_eq!(posted(&replay, 0, &["events", "0", "output"]), json("[]"));
    let event = posted_event(&replay, 0);
    assert!(matches!(&event, Json::Object(fields) if !fields.contains_key("metrics")));
}

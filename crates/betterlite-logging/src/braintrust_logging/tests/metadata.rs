use super::support::{Scenario, answer, json, logger, posted};
use crate::braintrust_logging::payload::CallbackPath;

#[tokio::test]
async fn string_metadata_extends_absent_keys() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.standard_logging_object = json(r#"{"model":"gpt-4o"}"#);
    scenario.metadata = json(r#"{"project_id":"p","user_id":"u-1"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    assert_eq!(
        posted(&replay, 0, &["events", "0", "metadata"]),
        json(r#"{"model":"gpt-4o","project_id":"p","user_id":"u-1"}"#)
    );
}

#[tokio::test]
async fn existing_metadata_keys_are_not_overwritten() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.standard_logging_object = json(r#"{"model":"gpt-4o","user_id":"original"}"#);
    scenario.metadata = json(r#"{"project_id":"p","user_id":"override"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    assert_eq!(
        posted(&replay, 0, &["events", "0", "metadata"]),
        json(r#"{"model":"gpt-4o","user_id":"original","project_id":"p"}"#)
    );
}

#[tokio::test]
async fn non_string_metadata_values_are_not_extended() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p","count":5,"flag":null,"items":["a"]}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    assert_eq!(
        posted(&replay, 0, &["events", "0", "metadata"]),
        json(r#"{"project_id":"p"}"#)
    );
}

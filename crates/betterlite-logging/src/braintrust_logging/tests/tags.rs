use betterlite_models::decisions::Json;

use super::support::{Scenario, answer, at_path, json, logger, posted_event};
use crate::braintrust_logging::payload::CallbackPath;

#[tokio::test]
async fn root_spans_carry_no_tags() {
    let (logger, replay) = logger(vec![answer("{}")], &["user_id"]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p","root_span_id":"root-1","user_id":"u-1"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert!(matches!(&event, Json::Object(fields) if !fields.contains_key("tags")));
}

#[tokio::test]
async fn tags_intersect_the_default_tag_list() {
    let (logger, replay) = logger(vec![answer("{}")], &["user_id", "missing"]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p","user_id":"u-1","session_id":"s-1"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert_eq!(at_path(&event, &["tags"]), &json(r#"["user_id:u-1"]"#));
}

#[tokio::test]
async fn tags_are_empty_without_a_root_span() {
    let (logger, replay) = logger(vec![answer("{}")], &["user_id"]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert_eq!(at_path(&event, &["tags"]), &json("[]"));
}

#[tokio::test]
async fn null_root_span_ids_still_carry_tags() {
    let (logger, replay) = logger(vec![answer("{}")], &["user_id"]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p","root_span_id":null,"user_id":"u-1"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert_eq!(at_path(&event, &["tags"]), &json(r#"["user_id:u-1"]"#));
}

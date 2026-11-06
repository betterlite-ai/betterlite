use betterlite_models::decisions::Json;

use super::support::{Scenario, answer, at_path, json, logger, posted_event};
use crate::braintrust_logging::event::CallbackPath;

#[tokio::test]
async fn span_name_defaults_to_chat_completion() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert_eq!(
        at_path(&event, &["span_attributes"]),
        &json(r#"{"name":"Chat Completion","type":"llm"}"#)
    );
}

#[tokio::test]
async fn span_attributes_and_ids_come_from_metadata() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(
        r#"{"project_id":"p","span_name":"custom span","span_id":"span-1","root_span_id":"root-1","span_parents":"parent-a, parent-b"}"#,
    );
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert_eq!(
        at_path(&event, &["span_attributes"]),
        &json(r#"{"name":"custom span","type":"llm"}"#)
    );
    assert_eq!(at_path(&event, &["span_id"]), &json(r#""span-1""#));
    assert_eq!(at_path(&event, &["root_span_id"]), &json(r#""root-1""#));
    assert_eq!(
        at_path(&event, &["span_parents"]),
        &json(r#"["parent-a","parent-b"]"#)
    );
}

#[tokio::test]
async fn span_parents_split_on_commas_and_skip_empty_entries() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"p","span_parents":" a, ,b ,"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert_eq!(at_path(&event, &["span_parents"]), &json(r#"["a","b"]"#));
}

#[tokio::test]
async fn empty_span_fields_are_omitted() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata =
        json(r#"{"project_id":"p","span_id":"","root_span_id":"","span_parents":" , "}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let event = posted_event(&replay, 0);
    assert!(matches!(&event, Json::Object(fields) if !fields.contains_key("span_id")));
    assert!(matches!(&event, Json::Object(fields) if !fields.contains_key("root_span_id")));
    assert!(matches!(&event, Json::Object(fields) if !fields.contains_key("span_parents")));
}

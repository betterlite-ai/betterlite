#![cfg(test)]

use crate::fields::payload::StandardLoggingPayload;

use super::tags::{add_default_langfuse_tags, get_langfuse_tags};

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": ["team-a", "team-b"],
    "model_parameters": {},
    "hidden_params": {}
}"#;

fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("a valid standard logging payload")
}

fn configured(tag: &str) -> Vec<String> {
    vec![tag.to_owned()]
}

#[test]
fn default_tags_come_from_the_standard_logging_payload() {
    assert_eq!(
        get_langfuse_tags(Some(&payload())),
        vec!["team-a".to_owned(), "team-b".to_owned()]
    );
    assert_eq!(get_langfuse_tags(None), Vec::<String>::new());
}

#[test]
fn cache_hit_tag_appends_the_flag_when_configured() {
    assert_eq!(
        add_default_langfuse_tags(Vec::new(), &configured("cache_hit"), Some(true), None, None),
        vec!["cache_hit:True".to_owned()]
    );
    assert_eq!(
        add_default_langfuse_tags(
            Vec::new(),
            &configured("cache_hit"),
            Some(false),
            None,
            None
        ),
        vec!["cache_hit:False".to_owned()]
    );
    assert_eq!(
        add_default_langfuse_tags(Vec::new(), &configured("cache_hit"), None, None, None),
        vec!["cache_hit:None".to_owned()]
    );
}

#[test]
fn cache_key_tag_prefers_the_hidden_params_key() {
    assert_eq!(
        add_default_langfuse_tags(
            Vec::new(),
            &configured("cache_key"),
            Some(false),
            Some("hidden-key"),
            Some("preset-key")
        ),
        vec!["cache_key:hidden-key".to_owned()]
    );
}

#[test]
fn cache_key_tag_falls_back_to_the_preset_cache_key() {
    assert_eq!(
        add_default_langfuse_tags(
            Vec::new(),
            &configured("cache_key"),
            Some(false),
            None,
            Some("preset-key")
        ),
        vec!["cache_key:preset-key".to_owned()]
    );
    assert_eq!(
        add_default_langfuse_tags(
            Vec::new(),
            &configured("cache_key"),
            Some(false),
            None,
            None
        ),
        vec!["cache_key:None".to_owned()]
    );
}

#[test]
fn default_tags_are_left_alone_without_the_setting() {
    let tags = vec!["existing".to_owned()];
    assert_eq!(
        add_default_langfuse_tags(tags.clone(), &[], Some(true), Some("key"), None),
        tags
    );
    assert_eq!(
        add_default_langfuse_tags(
            tags.clone(),
            &configured("another_tag"),
            Some(true),
            Some("key"),
            None
        ),
        tags
    );
}

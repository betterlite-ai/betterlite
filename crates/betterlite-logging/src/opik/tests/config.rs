#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_logging::opik::config::{
    create_usage_object, get_config_variable, read_config_section,
};
use betterlite_logging::opik::payload::classify_payload;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

fn obj(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("valid fixture JSON")
}

#[test]
fn config_priority_and_partial_usage() {
    let env = BTreeMap::from([(String::from("OPIK_URL_OVERRIDE"), String::from("env"))]);
    let file = BTreeMap::from([(String::from("url_override"), String::from("file"))]);
    assert_eq!(
        get_config_variable("url_override", Some("user"), &env, &file, Some("default")),
        Some(String::from("user"))
    );
    assert_eq!(
        get_config_variable("url_override", None, &env, &file, Some("default")),
        Some(String::from("env"))
    );
    assert_eq!(
        get_config_variable("missing", None, &env, &file, Some("default")),
        Some(String::from("default"))
    );
    let usage = create_usage_object(&obj(r#"{"prompt_tokens": 2, "completion_tokens": null}"#));
    assert_eq!(usage, obj(r#"{"prompt_tokens": 2}"#));
}

#[test]
fn config_parser_reads_only_opik_section() {
    let values = read_config_section(
        "[other]\nurl_override=wrong\n[opik]\nurl_override = file\napi_key=file-key\n",
    );
    assert_eq!(values.get("url_override"), Some(&String::from("file")));
    assert_eq!(values.get("api_key"), Some(&String::from("file-key")));
    assert!(!values.contains_key("missing"));
}

#[test]
fn payload_classification_removes_nulls() {
    let payload = vec![
        obj(r#"{"id": "trace", "key2": null}"#),
        obj(r#"{"id": "span", "type": "llm", "key2": null}"#),
    ];
    let (traces, spans) = classify_payload(&payload);
    assert_eq!(traces, vec![obj(r#"{"id": "trace"}"#)]);
    assert_eq!(spans, vec![obj(r#"{"id": "span", "type": "llm"}"#)]);
}

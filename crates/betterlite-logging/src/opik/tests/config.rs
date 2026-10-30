#![cfg(test)]

use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn config_priority_and_partial_usage() {
    let env = BTreeMap::from([(String::from("OPIK_URL_OVERRIDE"), String::from("env"))]);
    let file = BTreeMap::from([(String::from("url_override"), String::from("file"))]);
    assert_eq!(
        betterlite_logging::opik::get_config_variable(
            "url_override",
            Some("user"),
            &env,
            &file,
            Some("default")
        ),
        Some(String::from("user"))
    );
    assert_eq!(
        betterlite_logging::opik::get_config_variable(
            "url_override",
            None,
            &env,
            &file,
            Some("default")
        ),
        Some(String::from("env"))
    );
    assert_eq!(
        betterlite_logging::opik::get_config_variable(
            "missing",
            None,
            &env,
            &file,
            Some("default")
        ),
        Some(String::from("default"))
    );
    let usage = betterlite_logging::opik::create_usage_object(
        json!({"prompt_tokens": 2, "completion_tokens": null})
            .as_object()
            .unwrap(),
    );
    assert_eq!(
        usage,
        json!({"prompt_tokens": 2}).as_object().unwrap().clone()
    );
}

#[test]
fn config_parser_reads_only_opik_section() {
    let values = betterlite_logging::opik::read_config_section(
        "[other]\nurl_override=wrong\n[opik]\nurl_override = file\napi_key=file-key\n",
    );
    assert_eq!(values.get("url_override"), Some(&String::from("file")));
    assert_eq!(values.get("api_key"), Some(&String::from("file-key")));
    assert!(!values.contains_key("missing"));
}

#[test]
fn payload_classification_removes_nulls() {
    let payload = vec![
        json!({"id": "trace", "key2": null}),
        json!({"id": "span", "type": "llm", "key2": null}),
    ];
    let (traces, spans) = betterlite_logging::opik::classify_payload(&payload);
    assert_eq!(
        traces,
        vec![json!({"id": "trace"}).as_object().unwrap().clone()]
    );
    assert_eq!(
        spans,
        vec![
            json!({"id": "span", "type": "llm"})
                .as_object()
                .unwrap()
                .clone()
        ]
    );
}

#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_logging::opik::config::{create_usage_object, get_config_variable};
use betterlite_logging::opik::config_file::read_config_section;
use betterlite_logging::opik::payload::classify_payload;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

fn obj(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("valid fixture JSON")
}

#[test]
fn config_priority_reads_the_file_last() {
    let env = BTreeMap::from([(String::from("OPIK_URL_OVERRIDE"), String::from("env"))]);
    let file = "[opik]\nurl_override = file\n";
    assert_eq!(
        get_config_variable(
            "url_override",
            Some("user"),
            &env,
            Some(file),
            Some("default")
        ),
        Ok(Some(String::from("user")))
    );
    assert_eq!(
        get_config_variable("url_override", None, &env, Some(file), Some("default")),
        Ok(Some(String::from("env")))
    );
    assert_eq!(
        get_config_variable(
            "url_override",
            None,
            &BTreeMap::new(),
            Some(file),
            Some("default")
        ),
        Ok(Some(String::from("file")))
    );
    assert_eq!(
        get_config_variable(
            "missing",
            None,
            &BTreeMap::new(),
            Some(file),
            Some("default")
        ),
        Ok(Some(String::from("default")))
    );
    assert_eq!(
        get_config_variable("missing", None, &BTreeMap::new(), Some(file), None),
        Ok(None)
    );
}

#[test]
fn usage_object_fills_missing_counts_with_zero() {
    let usage = create_usage_object(&obj(r#"{"prompt_tokens": 3, "completion_tokens": null}"#));
    assert_eq!(
        usage,
        obj(r#"{"completion_tokens": 0, "prompt_tokens": 3, "total_tokens": 0}"#)
    );
}

#[test]
fn config_lookup_skips_a_bad_file_when_answered() {
    let broken = "[opik]\nbroken = 50%\n";
    let env = BTreeMap::from([(String::from("OPIK_URL_OVERRIDE"), String::from("env"))]);
    assert_eq!(
        get_config_variable(
            "url_override",
            Some("user"),
            &BTreeMap::new(),
            Some(broken),
            None
        ),
        Ok(Some(String::from("user")))
    );
    assert_eq!(
        get_config_variable("url_override", None, &env, Some(broken), None),
        Ok(Some(String::from("env")))
    );
}

#[test]
fn config_parser_reads_only_opik_section() {
    let values = read_config_section(
        "[other]\nurl_override=wrong\nother_only=value\n[opik]\nurl_override = file\napi_key=file-key\n#hash_line=sneaky\n;semi_line=sneaky2\n",
    )
    .expect("valid configuration");
    assert_eq!(values.get("url_override"), Some(&String::from("file")));
    assert_eq!(values.get("api_key"), Some(&String::from("file-key")));
    assert!(!values.contains_key("missing"));
    assert!(!values.contains_key("other_only"));
    assert!(!values.contains_key("#hash_line"));
    assert!(!values.contains_key(";semi_line"));
}

#[test]
fn config_parser_preserves_hashes_and_quotes_in_values() {
    let values =
        read_config_section("[opik]\nurl_override = https://host/path#frag\napi_key = \"token\"\n")
            .expect("valid configuration");
    assert_eq!(
        values.get("url_override"),
        Some(&String::from("https://host/path#frag"))
    );
    assert_eq!(values.get("api_key"), Some(&String::from("\"token\"")));
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

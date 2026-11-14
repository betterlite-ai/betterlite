#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::response;
use crate::model_armor::decisions::{
    DEFAULT_MAX_RECURSE_DEPTH, ModelArmorAPIError, REDACTED, build_api_error_detail,
    build_block_error_detail, build_logging_response, raise_if_fail_closed, redact_scanned_content,
};

fn redacted_text() -> GuardrailValue {
    GuardrailValue::Text(REDACTED.to_owned())
}

fn nested_text(levels: usize) -> GuardrailValue {
    let mut value = GuardrailValue::Text("marker".to_owned());
    for _ in 0..levels {
        let mut fields = BTreeMap::new();
        fields.insert("nested".to_owned(), value);
        value = GuardrailValue::Object(fields);
    }
    value
}

#[rstest]
#[case::text("text")]
#[case::sanitized_text("sanitizedText")]
#[case::findings("findings")]
#[case::malicious_uri_matched_items("maliciousUriMatchedItems")]
fn redact_scanned_content_replaces_every_scanned_key(#[case] key: &str) {
    let source = format!(r#"{{"{key}":"secret","safe":"kept"}}"#);
    let redacted = redact_scanned_content(&response(&source), 0);
    let GuardrailValue::Object(fields) = redacted else {
        panic!("redacted content is not an object");
    };
    assert_eq!(fields.get(key), Some(&redacted_text()));
    assert_eq!(
        fields.get("safe"),
        Some(&GuardrailValue::Text("kept".to_owned()))
    );
}

#[test]
fn redact_scanned_content_reaches_nested_objects() {
    let redacted = redact_scanned_content(&response(r#"{"a":{"text":"x","other":"z"}}"#), 0);
    let GuardrailValue::Object(fields) = redacted else {
        panic!("redacted content is not an object");
    };
    let Some(GuardrailValue::Object(nested)) = fields.get("a") else {
        panic!("the nested object is gone");
    };
    assert_eq!(nested.get("text"), Some(&redacted_text()));
    assert_eq!(
        nested.get("other"),
        Some(&GuardrailValue::Text("z".to_owned()))
    );
}

#[test]
fn redact_scanned_content_reaches_items_in_a_list() {
    let redacted = redact_scanned_content(
        &response(r#"{"items":[{"sanitizedText":"y","other":"z"}]}"#),
        0,
    );
    let GuardrailValue::Object(fields) = redacted else {
        panic!("redacted content is not an object");
    };
    let Some(GuardrailValue::Array(items)) = fields.get("items") else {
        panic!("the list is gone");
    };
    let Some(GuardrailValue::Object(item)) = items.first() else {
        panic!("the list item is gone");
    };
    assert_eq!(item.get("sanitizedText"), Some(&redacted_text()));
    assert_eq!(
        item.get("other"),
        Some(&GuardrailValue::Text("z".to_owned()))
    );
}

#[test]
fn redact_scanned_content_keeps_an_empty_object_and_scalars() {
    assert_eq!(redact_scanned_content(&response("{}"), 0), response("{}"));
    assert_eq!(
        redact_scanned_content(&response(r#""plain""#), 0),
        response(r#""plain""#)
    );
    assert_eq!(redact_scanned_content(&response("7"), 0), response("7"));
    assert_eq!(
        redact_scanned_content(&response("null"), 0),
        response("null")
    );
}

#[test]
fn redact_scanned_content_keeps_a_value_just_inside_the_recursion_cap() {
    let inside = nested_text(DEFAULT_MAX_RECURSE_DEPTH - 1);
    assert_eq!(redact_scanned_content(&inside, 0), inside);
}

#[test]
fn redact_scanned_content_redacts_a_value_at_the_recursion_cap() {
    let at_the_cap = redact_scanned_content(&nested_text(DEFAULT_MAX_RECURSE_DEPTH), 0);
    let encoded = serde_json::to_string(&at_the_cap).expect("json");
    assert!(!encoded.contains("marker"));
    assert!(encoded.contains(REDACTED));
}

#[test]
fn redact_scanned_content_handles_a_structure_past_the_cap() {
    let past_the_cap = redact_scanned_content(&nested_text(DEFAULT_MAX_RECURSE_DEPTH + 20), 0);
    let encoded = serde_json::to_string(&past_the_cap).expect("json");
    assert!(!encoded.contains("marker"));
    assert!(encoded.contains(REDACTED));
}

#[test]
fn build_logging_response_redacts_only_when_sanitizing() {
    let payload = response(r#"{"text":"secret","safe":"kept"}"#);
    assert_eq!(
        build_logging_response(&payload, true),
        response(r#"{"text":"[REDACTED]","safe":"kept"}"#)
    );
    assert_eq!(build_logging_response(&payload, false), payload);
}

#[test]
fn build_block_error_detail_includes_the_response_only_when_verbose() {
    let payload = response(r#"{"text":"secret"}"#);
    let hidden = build_block_error_detail("Content blocked by Model Armor", &payload, true);
    let hidden_keys: Vec<&str> = hidden.keys().map(String::as_str).collect();
    assert_eq!(hidden_keys, ["error"]);
    assert_eq!(
        hidden.get("error"),
        Some(&GuardrailValue::Text(
            "Content blocked by Model Armor".to_owned()
        ))
    );
    let shown = build_block_error_detail("Content blocked by Model Armor", &payload, false);
    let shown_keys: Vec<&str> = shown.keys().map(String::as_str).collect();
    assert_eq!(shown_keys, ["error", "model_armor_response"]);
    assert_eq!(shown.get("model_armor_response"), Some(&payload));
}

#[test]
fn build_api_error_detail_hides_the_body_only_when_sanitizing() {
    assert_eq!(
        build_api_error_detail(503, "upstream body", true),
        "Model Armor API error (upstream 503)"
    );
    assert_eq!(
        build_api_error_detail(503, "upstream body", false),
        "Model Armor API error (upstream 503): upstream body"
    );
}

#[test]
fn raise_if_fail_closed_follows_fail_on_error() {
    let error = ModelArmorAPIError {
        detail: "Model Armor API error (upstream 503)".to_owned(),
    };
    assert_eq!(
        raise_if_fail_closed(error.clone(), true).err(),
        Some(error.clone())
    );
    assert_eq!(raise_if_fail_closed(error, false), Ok(()));
}

#[test]
fn model_armor_api_error_prints_its_detail() {
    let error = ModelArmorAPIError {
        detail: "Model Armor API error (upstream 503)".to_owned(),
    };
    assert_eq!(error.detail, "Model Armor API error (upstream 503)");
    assert_eq!(error.to_string(), "Model Armor API error (upstream 503)");
    let as_std: &dyn Error = &error;
    assert_eq!(as_std.to_string(), "Model Armor API error (upstream 503)");
}

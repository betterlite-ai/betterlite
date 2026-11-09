use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fake::map;
use super::fake::object;
use super::fake::text;
use crate::custom_code::CustomCodeGuardrail;

#[test]
fn safe_request_data_keeps_only_the_five_keys_and_drops_credentials() {
    let request_data = map(vec![
        ("model", text("gpt-4o")),
        ("api_key", text("sk-secret")),
        ("headers", object(vec![("authorization", text("Bearer x"))])),
        (
            "metadata",
            object(vec![
                ("user_api_key_user_id", text("u1")),
                ("user_api_key_team_id", text("t1")),
                ("user_api_key_end_user_id", text("e1")),
                ("unrelated", text("kept inside metadata")),
            ]),
        ),
    ]);
    let prepared = CustomCodeGuardrail::prepare_safe_request_data(&request_data).to_value();
    let GuardrailValue::Object(safe) = prepared else {
        panic!("expected the safe request data to be an object");
    };
    let keys: Vec<&str> = safe.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec!["end_user_id", "metadata", "model", "team_id", "user_id"]
    );
    assert_eq!(safe.get("model"), Some(&text("gpt-4o")));
    assert_eq!(safe.get("user_id"), Some(&text("u1")));
    assert_eq!(safe.get("team_id"), Some(&text("t1")));
    assert_eq!(safe.get("end_user_id"), Some(&text("e1")));
    assert!(!safe.contains_key("api_key"));
    assert!(!safe.contains_key("headers"));
}

#[test]
fn safe_request_data_keeps_the_metadata_minus_the_projected_ids() {
    let request_data = map(vec![(
        "metadata",
        object(vec![
            ("user_api_key_user_id", text("u1")),
            ("request_id", text("req-1")),
        ]),
    )]);
    let prepared = CustomCodeGuardrail::prepare_safe_request_data(&request_data).to_value();
    let GuardrailValue::Object(safe) = prepared else {
        panic!("expected the safe request data to be an object");
    };
    let Some(GuardrailValue::Object(metadata)) = safe.get("metadata") else {
        panic!("expected the metadata object");
    };
    assert_eq!(metadata.get("request_id"), Some(&text("req-1")));
    assert_eq!(metadata.get("user_api_key_user_id"), Some(&text("u1")));
}

#[test]
fn litellm_metadata_overrides_metadata_key_by_key() {
    let request_data = map(vec![
        (
            "metadata",
            object(vec![
                ("user_api_key_user_id", text("from-metadata")),
                ("team", text("kept")),
            ]),
        ),
        (
            "litellm_metadata",
            object(vec![("user_api_key_user_id", text("from-litellm"))]),
        ),
    ]);
    let prepared = CustomCodeGuardrail::prepare_safe_request_data(&request_data).to_value();
    let GuardrailValue::Object(safe) = prepared else {
        panic!("expected the safe request data to be an object");
    };
    assert_eq!(safe.get("user_id"), Some(&text("from-litellm")));
    let Some(GuardrailValue::Object(metadata)) = safe.get("metadata") else {
        panic!("expected the metadata object");
    };
    assert_eq!(metadata.get("team"), Some(&text("kept")));
}

#[test]
fn missing_values_become_null_and_metadata_is_an_empty_object() {
    let prepared = CustomCodeGuardrail::prepare_safe_request_data(&BTreeMap::new()).to_value();
    assert_eq!(
        prepared,
        object(vec![
            ("model", GuardrailValue::Null),
            ("user_id", GuardrailValue::Null),
            ("team_id", GuardrailValue::Null),
            ("end_user_id", GuardrailValue::Null),
            ("metadata", object(vec![])),
        ])
    );
}

#[test]
fn a_metadata_bucket_that_is_not_an_object_counts_as_empty() {
    let request_data = map(vec![("metadata", text("not a bucket"))]);
    let prepared = CustomCodeGuardrail::prepare_safe_request_data(&request_data).to_value();
    let GuardrailValue::Object(safe) = prepared else {
        panic!("expected the safe request data to be an object");
    };
    assert_eq!(safe.get("metadata"), Some(&object(vec![])));
    assert_eq!(safe.get("user_id"), Some(&GuardrailValue::Null));
}

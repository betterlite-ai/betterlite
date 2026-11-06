#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::StandardLoggingPayload;

const MINIMAL: &str = r#"{
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
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

fn field<'payload>(payload: &'payload Json, name: &str) -> &'payload Json {
    match payload {
        Json::Object(fields) => fields.get(name).unwrap_or_else(|| panic!("missing {name}")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("payload is not an object")
        }
    }
}

fn object_len(value: &Json) -> usize {
    match value {
        Json::Object(fields) => fields.len(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object")
        }
    }
}

fn round_trip(encoded: &str) -> Json {
    let payload: StandardLoggingPayload = serde_json::from_str(encoded).unwrap();
    serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap()
}

#[test]
fn every_declared_key_survives_a_round_trip() {
    let payload = round_trip(MINIMAL);
    assert_eq!(
        field(&payload, "startTime"),
        &serde_json::from_str::<Json>("1704067200.0").unwrap()
    );
    assert_eq!(
        field(&payload, "status"),
        &serde_json::from_str::<Json>(r#""success""#).unwrap()
    );
    assert_eq!(object_len(field(&payload, "metadata")), 35);
    assert_eq!(object_len(field(&payload, "hidden_params")), 11);
    assert_eq!(object_len(field(&payload, "status_fields")), 2);
}

#[test]
fn absent_auth_and_spend_values_stay_null_instead_of_becoming_zeros() {
    let payload = round_trip(MINIMAL);
    let metadata = field(&payload, "metadata");
    assert_eq!(
        field(metadata, "user_api_key_spend"),
        &serde_json::from_str::<Json>("null").unwrap()
    );
    assert_eq!(
        field(metadata, "user_api_key_max_budget"),
        &serde_json::from_str::<Json>("null").unwrap()
    );
    assert_eq!(
        field(metadata, "applied_guardrails"),
        &serde_json::from_str::<Json>("null").unwrap()
    );
}

#[test]
fn a_partial_metadata_record_keeps_every_key() {
    let encoded = MINIMAL.replace(
        r#""metadata": {}"#,
        r#""metadata": {"user_api_key_hash": "abc", "user_api_key_spend": 1.5}"#,
    );
    let payload = round_trip(&encoded);
    let metadata = field(&payload, "metadata");
    assert_eq!(object_len(metadata), 35);
    assert_eq!(
        field(metadata, "user_api_key_hash"),
        &serde_json::from_str::<Json>(r#""abc""#).unwrap()
    );
    assert_eq!(
        field(metadata, "user_api_key_team_id"),
        &serde_json::from_str::<Json>("null").unwrap()
    );
}

#[test]
fn optional_keys_are_absent_unless_written() {
    let payload = round_trip(MINIMAL);
    match &payload {
        Json::Object(fields) => {
            assert!(!fields.contains_key("classifier_input"));
            assert!(!fields.contains_key("request_model_access_groups"));
            assert!(!fields.contains_key("zero_cost_diagnostic"));
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("payload is not an object")
        }
    }
}

#[test]
fn optional_keys_are_written_when_present() {
    let encoded = MINIMAL.replace(
        r#""request_tags": []"#,
        r#""request_tags": [], "request_model_access_groups": ["group-a"], "classifier_input": {"k": "v"}"#,
    );
    let payload = round_trip(&encoded);
    assert_eq!(
        field(&payload, "request_model_access_groups"),
        &serde_json::from_str::<Json>(r#"["group-a"]"#).unwrap()
    );
    assert_eq!(
        field(&payload, "classifier_input"),
        &serde_json::from_str::<Json>(r#"{"k":"v"}"#).unwrap()
    );
}

#[test]
fn an_error_information_record_keeps_every_declared_key() {
    let encoded = MINIMAL.replace(
        r#""model_parameters": {}"#,
        r#""error_information": {"error_code": "500"}, "model_parameters": {}"#,
    );
    let payload = round_trip(&encoded);
    let information = field(&payload, "error_information");
    assert_eq!(object_len(information), 13);
    assert_eq!(
        field(information, "error_message"),
        &serde_json::from_str::<Json>("null").unwrap()
    );
}

#[test]
fn a_guardrail_record_keeps_only_the_keys_the_hook_wrote() {
    let encoded = MINIMAL.replace(
        r#""model_parameters": {}"#,
        r#""guardrail_information": [{"guardrail_name": "pii", "guardrail_status": "blocked"}], "model_parameters": {}"#,
    );
    let payload = round_trip(&encoded);
    let guardrails = field(&payload, "guardrail_information");
    let Json::List(entries) = guardrails else {
        panic!("guardrail_information is not a list")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(object_len(&entries[0]), 2);
}

#[test]
fn a_zero_cost_diagnostic_keeps_its_three_keys() {
    let encoded = MINIMAL.replace(
        r#""model_parameters": {}"#,
        r#""zero_cost_diagnostic": {"reason": "missing_pricing_key", "pricing_model": "m", "missing_pricing_keys": ["input_cost_per_token"]}, "model_parameters": {}"#,
    );
    let payload = round_trip(&encoded);
    assert_eq!(object_len(field(&payload, "zero_cost_diagnostic")), 3);
}

#[rstest]
#[case::unknown_key(MINIMAL.replace(r#""model": "gpt-4o""#, r#""model": "gpt-4o", "nope": 1"#))]
#[case::missing_required_key(MINIMAL.replace(r#""model": "gpt-4o","#, ""))]
#[case::wrong_status(MINIMAL.replace(r#""status": "success""#, r#""status": "ok""#))]
fn logging_fields_behavior_tests_behavior(#[case] encoded: String) {
    let _error = serde_json::from_str::<StandardLoggingPayload>(&encoded).unwrap_err();
}

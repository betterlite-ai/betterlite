use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;

use super::support::params;
use crate::api::response_id_codecs::response::build_response_id;
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::requested::requested_optional_params;

#[test]
fn nulls_and_non_optional_keys_are_dropped_and_explicit_false_stays() {
    let mut request = params(
        r#"{"store": false, "temperature": null, "unknown": 5, "input": "hi", "model": "m"}"#,
    );
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"store": false}"#));
}

#[test]
fn control_keys_are_popped_from_the_caller_map() {
    let mut request = params(
        r#"{"custom_llm_provider": "openai", "kwargs": {}, "additional_drop_params": [], "store": true}"#,
    );
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"store": true}"#));
    assert_eq!(request, params(r#"{"store": true}"#));
}

#[test]
fn special_params_are_merged_before_the_key_filter() {
    let mut request = params(r#"{"kwargs": {"store": true, "not_a_response_key": 1}}"#);
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"store": true}"#));
}

#[test]
fn provider_specific_special_params_are_dropped() {
    let mut request = params(r#"{"kwargs": {"aws_bedrock_project_id": "p", "store": true}}"#);
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"store": true}"#));
}

#[test]
fn a_top_level_additional_drop_removes_the_key() {
    let mut request = params(r#"{"store": true, "additional_drop_params": ["store"]}"#);
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert!(selected.is_empty(), "{selected:?}");
}

#[test]
fn a_nested_additional_drop_removes_the_nested_key() {
    let mut request = params(
        r#"{"reasoning": {"effort": "high", "summary": "auto"}, "additional_drop_params": ["reasoning.summary"]}"#,
    );
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"reasoning": {"effort": "high"}}"#));
}

#[test]
fn an_encoded_previous_response_id_is_restored() {
    let encoded = build_response_id(Some("openai"), Some("model-id"), "resp_a");
    let mut request = params(&format!(r#"{{"previous_response_id": "{encoded}"}}"#));
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"previous_response_id": "resp_a"}"#));
}

#[test]
fn a_plain_previous_response_id_stays() {
    let mut request = params(r#"{"previous_response_id": "resp_a"}"#);
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"previous_response_id": "resp_a"}"#));
}

#[test]
fn metadata_keeps_only_string_values_and_hides_hidden_params() {
    let mut request =
        params(r#"{"metadata": {"a": "x", "hidden_params": {"k": "v"}, "n": 3, "b": "y"}}"#);
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"metadata": {"a": "x", "b": "y"}}"#));
}

#[test]
fn metadata_keeps_the_first_sixteen_string_values() {
    let fields: Vec<String> = (0_usize..18)
        .map(|index| format!("\"key{index:02}\": \"value{index:02}\""))
        .collect();
    let mut request = params(&format!(r#"{{"metadata": {{{}}}}}"#, fields.join(", ")));
    let selected = requested_optional_params(&mut request).expect("valid params");
    let JsonValue::Object(metadata) = selected.get("metadata").expect("metadata") else {
        panic!("metadata must stay an object");
    };
    assert_eq!(metadata.len(), 16);
    assert!(metadata.contains_key("key15"));
    assert!(!metadata.contains_key("key16"));
}

#[rstest]
#[case::text(r#"{"metadata": "not-an-object"}"#)]
#[case::list(r#"{"metadata": [1]}"#)]
#[case::null(r#"{"metadata": null}"#)]
fn metadata_that_converts_to_null_is_removed(#[case] source: &str) {
    let mut request = params(source);
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert!(selected.is_empty(), "{selected:?}");
}

#[test]
fn an_empty_metadata_object_stays() {
    let mut request = params(r#"{"metadata": {}}"#);
    let selected = requested_optional_params(&mut request).expect("valid params");
    assert_eq!(selected, params(r#"{"metadata": {}}"#));
}

#[rstest]
#[case::provider(r#"{"custom_llm_provider": 1}"#)]
#[case::kwargs(r#"{"kwargs": []}"#)]
#[case::additional(r#"{"additional_drop_params": "store"}"#)]
#[case::additional_entry(r#"{"additional_drop_params": [1]}"#)]
fn a_wrong_control_key_type_is_an_error(#[case] source: &str) {
    let mut request = params(source);
    let error = requested_optional_params(&mut request).expect_err("a wrong type must fail");
    assert!(error.status_code().is_none(), "not an SDK status");
    assert!(matches!(
        error,
        ResponsesParametersError::NotText { .. }
            | ResponsesParametersError::NotAnObject { .. }
            | ResponsesParametersError::NotList { .. }
    ));
}

#[test]
fn the_unrelated_parameter_map_is_left_alone() {
    let mut request = params(r#"{"store": true, "unknown": 5}"#);
    requested_optional_params(&mut request).expect("valid params");
    assert_eq!(request, params(r#"{"store": true, "unknown": 5}"#));
}

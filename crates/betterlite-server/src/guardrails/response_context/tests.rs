#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::add_response_context;

fn body(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

#[test]
fn response_is_inserted_under_its_key_when_missing() {
    let response = value(r#"{"choices":[{"text":"a"}]}"#);
    let mut request_data = body(r#"{"model":"gpt"}"#);
    add_response_context(&mut request_data, &response, None);
    assert_eq!(request_data.get("response"), Some(&response));
    assert_eq!(
        request_data.get("model"),
        Some(&GuardrailValue::Text("gpt".to_owned()))
    );
}

#[test]
fn an_existing_response_value_is_kept_even_null() {
    let mut request_data = body(r#"{"response":null}"#);
    add_response_context(&mut request_data, &value(r#"{"choices":[]}"#), None);
    assert_eq!(request_data.get("response"), Some(&GuardrailValue::Null));
}

#[test]
fn user_metadata_is_prefixed_and_inserted_as_an_object() {
    let user_api_key_dict = body(r#"{"user_id":"u-1","team_id":"t-1"}"#);
    let mut request_data = BTreeMap::new();
    add_response_context(
        &mut request_data,
        &value(r#"{"choices":[]}"#),
        Some(&user_api_key_dict),
    );
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&value(
            r#"{"user_api_key_team_id":"t-1","user_api_key_user_id":"u-1"}"#
        ))
    );
}

#[test]
fn prefixed_user_metadata_keys_are_kept_as_they_are() {
    let user_api_key_dict = body(r#"{"user_api_key_user_id":"u-1"}"#);
    let mut request_data = BTreeMap::new();
    add_response_context(
        &mut request_data,
        &value(r#"{"choices":[]}"#),
        Some(&user_api_key_dict),
    );
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&value(r#"{"user_api_key_user_id":"u-1"}"#))
    );
}

#[test]
fn null_values_and_underscore_keys_are_dropped() {
    let user_api_key_dict = body(r#"{"user_id":null,"_internal":"hidden","ok":"kept"}"#);
    let mut request_data = BTreeMap::new();
    add_response_context(
        &mut request_data,
        &value(r#"{"choices":[]}"#),
        Some(&user_api_key_dict),
    );
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&value(r#"{"user_api_key_ok":"kept"}"#))
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some("{}"))]
#[case::all_dropped(Some(r#"{"user_id":null,"_internal":"hidden"}"#))]
fn an_empty_result_adds_no_litellm_metadata(#[case] user_api_key_dict: Option<&str>) {
    let user_metadata = user_api_key_dict.map(body);
    let mut request_data = BTreeMap::new();
    add_response_context(
        &mut request_data,
        &value(r#"{"choices":[]}"#),
        user_metadata.as_ref(),
    );
    assert!(!request_data.contains_key("litellm_metadata"));
}

#[test]
fn an_existing_litellm_metadata_key_blocks_the_user_metadata() {
    let user_api_key_dict = body(r#"{"user_id":"u-1"}"#);
    let mut request_data = body(r#"{"litellm_metadata":null}"#);
    add_response_context(
        &mut request_data,
        &value(r#"{"choices":[]}"#),
        Some(&user_api_key_dict),
    );
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&GuardrailValue::Null)
    );
}

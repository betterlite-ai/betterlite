use betterlite_context::non_default_params::Params;
use rstest::rstest;

use crate::api::interactions::utils::optional_params::requested_optional_params;

fn parse_params(source: &str) -> Params {
    serde_json::from_str(source).expect("the fixture should parse")
}

#[rstest]
#[case::named_fields(
    r#"{
        "tools": [{"type": "google_search"}],
        "system_instruction": "be brief",
        "generation_config": {"temperature": 0.2},
        "stream": true,
        "store": true,
        "background": true,
        "environment": "remote",
        "response_modalities": ["TEXT"],
        "response_format": {"type": "text"},
        "response_mime_type": "text/plain",
        "previous_interaction_id": "int_0",
        "agent_config": {"type": "dynamic"}
    }"#
)]
#[case::false_and_empty(
    r#"{
        "stream": false,
        "store": false,
        "background": false,
        "environment": "",
        "response_modalities": [],
        "response_format": {},
        "response_mime_type": "",
        "previous_interaction_id": "",
        "tools": [],
        "system_instruction": "",
        "generation_config": {},
        "agent_config": {"type": "dynamic"}
    }"#
)]
fn keeps_requested_optional_params(#[case] source: &str) {
    let params = parse_params(source);
    assert_eq!(
        requested_optional_params(&params).expect("the optional params filter"),
        params
    );
}

#[test]
fn drops_null_and_unsupported_fields() {
    let params = parse_params(
        r#"{
            "model": "gemini-2.5-flash",
            "agent": "agent_1",
            "input": "hi",
            "custom_llm_provider": "gemini",
            "api_version": "v1beta",
            "drop_params": true,
            "allowed_openai_params": [],
            "additional_drop_params": [],
            "messages": [],
            "top_p": 0.9,
            "stream": null,
            "store": false
        }"#,
    );
    assert_eq!(
        requested_optional_params(&params).expect("the optional params filter"),
        parse_params(r#"{"store": false}"#)
    );
}

#[test]
fn merges_kwargs_into_the_optional_params() {
    let params = parse_params(r#"{"kwargs": {"store": false, "background": true, "top_p": 0.5}}"#);
    assert_eq!(
        requested_optional_params(&params).expect("the optional params filter"),
        parse_params(r#"{"store": false, "background": true}"#)
    );
}

#[test]
fn additional_drop_params_removes_named_fields() {
    let params =
        parse_params(r#"{"store": true, "background": true, "additional_drop_params": ["store"]}"#);
    assert_eq!(
        requested_optional_params(&params).expect("the optional params filter"),
        parse_params(r#"{"background": true}"#)
    );
}

#[test]
fn absent_control_fields_do_not_change_the_result() {
    let params = parse_params(r#"{"tools": []}"#);
    assert_eq!(
        requested_optional_params(&params).expect("the optional params filter"),
        parse_params(r#"{"tools": []}"#)
    );
}

#[test]
fn null_control_fields_are_unset() {
    let params = parse_params(
        r#"{"custom_llm_provider": null, "kwargs": null, "additional_drop_params": null, "store": false}"#,
    );
    assert_eq!(
        requested_optional_params(&params).expect("the optional params filter"),
        parse_params(r#"{"store": false}"#)
    );
}

#[rstest]
#[case::provider_bool(
    r#"{"custom_llm_provider": true}"#,
    "custom_llm_provider must be a string"
)]
#[case::provider_number(
    r#"{"custom_llm_provider": 7}"#,
    "custom_llm_provider must be a string"
)]
#[case::provider_list(
    r#"{"custom_llm_provider": ["gemini"]}"#,
    "custom_llm_provider must be a string"
)]
#[case::provider_object(
    r#"{"custom_llm_provider": {"name": "gemini"}}"#,
    "custom_llm_provider must be a string"
)]
#[case::kwargs_text(r#"{"kwargs": "store"}"#, "kwargs must be an object")]
#[case::kwargs_bool(r#"{"kwargs": false}"#, "kwargs must be an object")]
#[case::kwargs_list(r#"{"kwargs": [["store", false]]}"#, "kwargs must be an object")]
#[case::kwargs_number(r#"{"kwargs": 1}"#, "kwargs must be an object")]
#[case::drop_params_text(
    r#"{"additional_drop_params": "store"}"#,
    "additional_drop_params must be a list"
)]
#[case::drop_params_object(
    r#"{"additional_drop_params": {"store": true}}"#,
    "additional_drop_params must be a list"
)]
#[case::drop_params_bool(
    r#"{"additional_drop_params": true}"#,
    "additional_drop_params must be a list"
)]
#[case::drop_params_number(
    r#"{"additional_drop_params": 3}"#,
    "additional_drop_params must be a list"
)]
#[case::drop_params_null_item(
    r#"{"additional_drop_params": [null]}"#,
    "additional_drop_params must be a string"
)]
#[case::drop_params_number_item(
    r#"{"additional_drop_params": [7]}"#,
    "additional_drop_params must be a string"
)]
#[case::drop_params_object_item(
    r#"{"additional_drop_params": [{"store": true}]}"#,
    "additional_drop_params must be a string"
)]
#[case::drop_params_list_item(
    r#"{"additional_drop_params": [["store"]]}"#,
    "additional_drop_params must be a string"
)]
fn rejects_wrongly_typed_optional_control_fields(#[case] source: &str, #[case] message: &str) {
    let error = requested_optional_params(&parse_params(source))
        .expect_err("the control field has the wrong type");
    assert_eq!(error.to_string(), message);
}

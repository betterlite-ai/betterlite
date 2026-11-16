#![cfg(test)]

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::container_contracts::records::{
    ContainerCreateOptionalParams, ContainerCreateRequestParams, ContainerListOptionalParams,
};
use indexmap::IndexMap;
use rstest::rstest;

use crate::api::response_id_codecs::container::build_container_id;
use crate::containers::container_request_parameters::{
    ContainerProviderConfig, decode_managed_container_id_for_request,
    encode_container_id_in_response, get_optional_params_container_create,
    get_provider_container_config, get_requested_container_create_optional_param,
    get_requested_container_list_optional_param,
};

fn json_map(text: &str) -> IndexMap<String, JsonValue> {
    serde_json::from_str(text).expect("a JSON object")
}

fn create_request(text: &str) -> ContainerCreateRequestParams {
    serde_json::from_str(text).expect("a create request")
}

fn create_optional(text: &str) -> ContainerCreateOptionalParams {
    serde_json::from_str(text).expect("create optional parameters")
}

fn list_optional(text: &str) -> ContainerListOptionalParams {
    serde_json::from_str(text).expect("list optional parameters")
}

#[test]
fn decode_managed_container_id_restores_the_original_id_and_provider() {
    let container_id = build_container_id(Some("azure"), Some("dep_a"), "container_123");
    let decoded = decode_managed_container_id_for_request(&container_id, "openai", &json_map("{}"))
        .expect("a managed id decodes");
    assert_eq!(decoded.container_id, "container_123");
    assert_eq!(decoded.custom_llm_provider, "azure");
}

#[rstest]
#[case::absent("{}", "dep_a")]
#[case::null(r#"{"model_id": null}"#, "dep_a")]
#[case::preset(r#"{"model_id": "preset"}"#, "preset")]
#[case::empty(r#"{"model_id": ""}"#, "")]
fn decode_managed_container_id_fills_or_keeps_the_model_id(
    #[case] params: &str,
    #[case] expected: &str,
) {
    let container_id = build_container_id(Some("azure"), Some("dep_a"), "container_123");
    let decoded =
        decode_managed_container_id_for_request(&container_id, "openai", &json_map(params))
            .expect("a managed id decodes");
    let expected_model_id = JsonValue::Text(expected.to_owned());
    assert_eq!(
        decoded.litellm_params.get("model_id"),
        Some(&expected_model_id)
    );
}

#[test]
fn decode_managed_container_id_adds_no_model_id_without_one_in_the_id() {
    let container_id = build_container_id(Some("azure"), None, "container_123");
    let decoded = decode_managed_container_id_for_request(&container_id, "openai", &json_map("{}"))
        .expect("a managed id decodes");
    assert_eq!(decoded.litellm_params, json_map("{}"));
}

#[rstest]
#[case::openai_is_overridden("openai", Some("azure"), "azure")]
#[case::azure_text_keeps_its_own("azure_text", Some("azure"), "azure_text")]
#[case::plain_openai_stays_when_the_id_names_none("openai", None, "openai")]
fn decode_managed_container_id_resolves_the_provider(
    #[case] input_provider: &str,
    #[case] encoded_provider: Option<&str>,
    #[case] expected: &str,
) {
    let container_id = build_container_id(encoded_provider, Some("dep_a"), "container_123");
    let decoded =
        decode_managed_container_id_for_request(&container_id, input_provider, &json_map("{}"))
            .expect("a managed id decodes");
    assert_eq!(decoded.custom_llm_provider, expected);
}

#[test]
fn decode_managed_container_id_passes_an_unmanaged_id_through() {
    let params = json_map(r#"{"api_key": "sk-test"}"#);
    let decoded = decode_managed_container_id_for_request("container_raw", "azure", &params)
        .expect("an unmanaged id passes through");
    assert_eq!(decoded.container_id, "container_raw");
    assert_eq!(decoded.custom_llm_provider, "azure");
    assert_eq!(decoded.litellm_params, params);
}

#[test]
fn decode_managed_container_id_rejects_a_non_text_model_id() {
    let container_id = build_container_id(Some("azure"), Some("dep_a"), "container_123");
    let error = decode_managed_container_id_for_request(
        &container_id,
        "openai",
        &json_map(r#"{"model_id": 5}"#),
    )
    .expect_err("a non-text model_id is refused");
    assert_eq!(error.field, "litellm_params.model_id");
}

#[test]
fn decode_managed_container_id_ignores_a_non_text_model_id_without_a_decoded_one() {
    let container_id = build_container_id(Some("azure"), None, "container_123");
    let decoded = decode_managed_container_id_for_request(
        &container_id,
        "openai",
        &json_map(r#"{"model_id": 5}"#),
    )
    .expect("an unused model_id is never read");
    assert_eq!(decoded.litellm_params, json_map(r#"{"model_id": 5}"#));
}

#[test]
fn extract_create_optional_parameters_keeps_the_optional_ones() {
    let params = create_request(
        r#"{"name": "n", "expires_after": {"anchor": "last_active_at", "minutes": 30}, "file_ids": ["file_1"], "extra_headers": {"X-A": "b"}, "extra_body": {"k": "v"}}"#,
    );
    let expected = create_optional(
        r#"{"expires_after": {"anchor": "last_active_at", "minutes": 30}, "file_ids": ["file_1"], "extra_headers": {"X-A": "b"}, "extra_body": {"k": "v"}}"#,
    );
    assert_eq!(
        get_requested_container_create_optional_param(&params),
        expected
    );
}

#[test]
fn extract_create_optional_parameters_drops_absent_and_null_values() {
    let params = create_request(
        r#"{"name": "n", "expires_after": null, "file_ids": null, "extra_headers": {"X-A": "b"}}"#,
    );
    assert_eq!(
        get_requested_container_create_optional_param(&params),
        create_optional(r#"{"extra_headers": {"X-A": "b"}}"#)
    );
}

#[test]
fn extract_create_optional_parameters_keeps_explicit_empty_values() {
    let params =
        create_request(r#"{"name": "n", "file_ids": [], "extra_headers": {}, "extra_body": {}}"#);
    assert_eq!(
        get_requested_container_create_optional_param(&params),
        create_optional(r#"{"file_ids": [], "extra_headers": {}, "extra_body": {}}"#)
    );
}

#[test]
fn extract_create_optional_parameters_ignores_unknown_parameters() {
    let params = create_request(r#"{"name": "n", "metadata": {"a": "b"}, "timeout": 5}"#);
    assert_eq!(
        get_requested_container_create_optional_param(&params),
        create_optional("{}")
    );
}

#[test]
fn filter_create_params_drops_extra_body() {
    let params = create_optional(
        r#"{"expires_after": {"anchor": "last_active_at", "minutes": 30}, "file_ids": ["file_1"], "extra_headers": {"X-A": "b"}, "extra_body": {"k": "v"}}"#,
    );
    let expected = create_optional(
        r#"{"expires_after": {"anchor": "last_active_at", "minutes": 30}, "file_ids": ["file_1"], "extra_headers": {"X-A": "b"}}"#,
    );
    assert_eq!(get_optional_params_container_create(&params), expected);
}

#[test]
fn filter_create_params_keeps_the_supported_parameters() {
    let params = create_optional(
        r#"{"expires_after": {"anchor": "last_active_at", "minutes": 30}, "file_ids": ["file_1"]}"#,
    );
    assert_eq!(get_optional_params_container_create(&params), params);
}

#[test]
fn filter_create_params_returns_an_empty_set_when_only_extra_body_is_set() {
    let params = create_optional(r#"{"extra_body": {"k": "v"}}"#);
    assert_eq!(
        get_optional_params_container_create(&params),
        create_optional("{}")
    );
}

#[test]
fn extract_list_optional_parameters_keeps_the_list_parameters() {
    let params = list_optional(
        r#"{"after": "cursor_1", "limit": 20, "order": "desc", "extra_headers": {"X-A": "b"}, "extra_query": {"q": "1"}}"#,
    );
    assert_eq!(
        get_requested_container_list_optional_param(&params),
        list_optional(
            r#"{"after": "cursor_1", "limit": 20, "order": "desc", "extra_headers": {"X-A": "b"}, "extra_query": {"q": "1"}}"#
        )
    );
}

#[test]
fn extract_list_optional_parameters_drops_absent_values() {
    let params = list_optional(r#"{"limit": 20}"#);
    assert_eq!(
        get_requested_container_list_optional_param(&params),
        list_optional(r#"{"limit": 20}"#)
    );
}

#[test]
fn extract_list_optional_parameters_keeps_explicit_empty_values() {
    let params = list_optional(
        r#"{"after": "", "order": "", "extra_headers": {}, "extra_query": {}, "limit": 0}"#,
    );
    assert_eq!(
        get_requested_container_list_optional_param(&params),
        list_optional(
            r#"{"after": "", "order": "", "extra_headers": {}, "extra_query": {}, "limit": 0}"#
        )
    );
}

#[test]
fn encode_container_id_uses_the_model_info_id() {
    let metadata = json_map(r#"{"model_info": {"id": "dep_a"}}"#);
    let encoded =
        encode_container_id_in_response("container_1", Some("azure"), Some(&metadata), None)
            .expect("a text model id encodes");
    assert_eq!(
        encoded,
        build_container_id(Some("azure"), Some("dep_a"), "container_1")
    );
}

#[test]
fn encode_container_id_uses_the_first_target_model_name() {
    let extra_body = json_map(r#"{"target_model_names": " m1 , m2"}"#);
    let encoded =
        encode_container_id_in_response("container_1", Some("azure"), None, Some(&extra_body))
            .expect("a target model name encodes");
    assert_eq!(
        encoded,
        build_container_id(Some("azure"), Some("m1"), "container_1")
    );
}

#[test]
fn encode_container_id_uses_the_first_target_model_from_a_list() {
    let extra_body = json_map(r#"{"target_model_names": [" m3 ", "m4"]}"#);
    let encoded =
        encode_container_id_in_response("container_1", Some("azure"), None, Some(&extra_body))
            .expect("a target model list encodes");
    assert_eq!(
        encoded,
        build_container_id(Some("azure"), Some("m3"), "container_1")
    );
}

#[test]
fn encode_container_id_returns_the_raw_id_without_routing_metadata() {
    let metadata = json_map(r#"{"model_info": {}}"#);
    let extra_body = json_map(r#"{"other": 1}"#);
    assert_eq!(
        encode_container_id_in_response("container_1", Some("azure"), Some(&metadata), None)
            .expect("an empty model_info encodes nothing"),
        "container_1"
    );
    assert_eq!(
        encode_container_id_in_response("container_1", Some("azure"), None, Some(&extra_body))
            .expect("an unrelated extra_body encodes nothing"),
        "container_1"
    );
    assert_eq!(
        encode_container_id_in_response("container_1", Some("azure"), None, None)
            .expect("a direct SDK call encodes nothing"),
        "container_1"
    );
}

#[test]
fn encode_container_id_prefers_the_model_info_id_over_target_model_names() {
    let metadata = json_map(r#"{"model_info": {"id": "dep_a"}}"#);
    let extra_body = json_map(r#"{"target_model_names": "m1"}"#);
    let encoded = encode_container_id_in_response(
        "container_1",
        Some("azure"),
        Some(&metadata),
        Some(&extra_body),
    )
    .expect("the router model id wins");
    assert_eq!(
        encoded,
        build_container_id(Some("azure"), Some("dep_a"), "container_1")
    );
}

#[test]
fn encode_container_id_ignores_target_model_names_with_a_model_info_id() {
    let metadata = json_map(r#"{"model_info": {"id": "dep_a"}}"#);
    let extra_body = json_map(r#"{"target_model_names": 5}"#);
    let encoded = encode_container_id_in_response(
        "container_1",
        Some("azure"),
        Some(&metadata),
        Some(&extra_body),
    )
    .expect("the router model id leaves target_model_names unread");
    assert_eq!(
        encoded,
        build_container_id(Some("azure"), Some("dep_a"), "container_1")
    );
}

#[test]
fn encode_container_id_encodes_an_empty_model_info_id() {
    let metadata = json_map(r#"{"model_info": {"id": ""}}"#);
    let encoded =
        encode_container_id_in_response("container_1", Some("azure"), Some(&metadata), None)
            .expect("an explicit empty id still encodes");
    assert_eq!(
        encoded,
        build_container_id(Some("azure"), Some(""), "container_1")
    );
}

#[test]
fn encode_container_id_encodes_an_empty_target_model_list() {
    let extra_body = json_map(r#"{"target_model_names": []}"#);
    let encoded =
        encode_container_id_in_response("container_1", Some("azure"), None, Some(&extra_body))
            .expect("a present target list encodes without a model");
    assert_eq!(
        encoded,
        build_container_id(Some("azure"), None, "container_1")
    );
}

#[test]
fn encode_container_id_encodes_without_a_provider() {
    let metadata = json_map(r#"{"model_info": {"id": "dep_a"}}"#);
    let encoded = encode_container_id_in_response("container_1", None, Some(&metadata), None)
        .expect("a missing provider encodes as empty");
    assert_eq!(
        encoded,
        build_container_id(None, Some("dep_a"), "container_1")
    );
}

#[test]
fn encode_container_id_rejects_a_non_object_model_info() {
    let error = encode_container_id_in_response(
        "container_1",
        Some("azure"),
        Some(&json_map(r#"{"model_info": 5}"#)),
        None,
    )
    .expect_err("a non-object model_info is refused");
    assert_eq!(error.field, "litellm_metadata.model_info");
}

#[test]
fn encode_container_id_rejects_a_non_text_model_info_id() {
    let error = encode_container_id_in_response(
        "container_1",
        Some("azure"),
        Some(&json_map(r#"{"model_info": {"id": 5}}"#)),
        None,
    )
    .expect_err("a non-text model id is refused");
    assert_eq!(error.field, "litellm_metadata.model_info.id");
}

#[test]
fn encode_container_id_rejects_a_non_text_target_model_names() {
    let error = encode_container_id_in_response(
        "container_1",
        Some("azure"),
        None,
        Some(&json_map(r#"{"target_model_names": 5}"#)),
    )
    .expect_err("a non-text target model names is refused");
    assert_eq!(error.field, "extra_body.target_model_names");
}

#[test]
fn encode_container_id_rejects_a_non_text_target_model() {
    let error = encode_container_id_in_response(
        "container_1",
        Some("azure"),
        None,
        Some(&json_map(r#"{"target_model_names": [5]}"#)),
    )
    .expect_err("a non-text target model is refused");
    assert_eq!(error.field, "extra_body.target_model_names");
}

#[test]
fn provider_config_selects_openai() {
    assert_eq!(
        get_provider_container_config("openai"),
        Some(ContainerProviderConfig::OpenAi)
    );
}

#[rstest]
#[case::azure("azure")]
#[case::azure_text("azure_text")]
fn provider_config_selects_azure(#[case] provider: &str) {
    assert_eq!(
        get_provider_container_config(provider),
        Some(ContainerProviderConfig::Azure)
    );
}

#[rstest]
#[case::empty("")]
#[case::capitalized("OpenAI")]
#[case::anthropic("anthropic")]
fn provider_config_returns_none_for_other_providers(#[case] provider: &str) {
    assert_eq!(get_provider_container_config(provider), None);
}

#![cfg(test)]

use std::collections::BTreeSet;

use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Value;

use super::capabilities::{CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag};
use super::classification::{ParameterClass, classify_parameter};
use super::{
    CallType, ChatParameters, ParameterError, ParameterMapping, ParameterPolicy, ParameterRequest,
    TYPED_PARAMETER_NAMES, map_parameters,
};
use crate::identity::provider_id::ProviderId;

fn parameters(source: &str) -> ChatParameters {
    serde_json::from_str(source).unwrap()
}

fn policy(source: &str) -> ParameterPolicy {
    serde_json::from_str(source).unwrap()
}

fn value(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn caps(names: &[OpenAiCapability]) -> BTreeSet<OpenAiCapability> {
    names.iter().copied().collect()
}

fn query<'model>(
    model: &'model str,
    config: OpenAiConfigTag,
    capabilities: &'model BTreeSet<OpenAiCapability>,
) -> CapabilityQuery<'model> {
    CapabilityQuery {
        provider: ProviderId::Openai,
        call_type: CallType::ChatCompletion,
        config,
        model: ModelProjection {
            model,
            capabilities,
        },
        base_model: None,
    }
}

fn mapping(
    parameters_source: &str,
    policy_source: &str,
) -> Result<ParameterMapping, ParameterError> {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(parameters_source);
    let policy = policy(policy_source);
    map_parameters(&ParameterRequest {
        capabilities: query("gpt-4o", OpenAiConfigTag::Gpt, &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
}

fn supported_parameters_are_separated() {
    let mapping = mapping(
        r#"{"temperature": 0.5, "logprobs": true, "vendor_flag": {"mode": "fast"}}"#,
        "{}",
    )
    .unwrap();
    assert_eq!(mapping.standard.len(), 2);
    assert_eq!(mapping.standard["temperature"], value("0.5"));
    assert_eq!(mapping.standard["logprobs"], value("true"));
    assert_eq!(mapping.provider_fields.len(), 1);
    assert_eq!(
        mapping.provider_fields["vendor_flag"],
        value(r#"{"mode": "fast"}"#)
    );
    assert!(mapping.dropped_fields.is_empty());
}

fn explicit_zero_and_empty_values_survive() {
    let mapping = mapping(
        r#"{"temperature": 0, "n": 0, "stop": "", "user": "", "stream": false, "logprobs": false}"#,
        "{}",
    )
    .unwrap();
    assert_eq!(mapping.standard["temperature"], value("0"));
    assert_eq!(mapping.standard["n"], value("0"));
    assert_eq!(mapping.standard["stop"], value(r#""""#));
    assert_eq!(mapping.standard["user"], value(r#""""#));
    assert_eq!(mapping.standard["stream"], value("false"));
    assert_eq!(mapping.standard["logprobs"], value("false"));
    assert_eq!(mapping.standard.len(), 6);
}

fn null_values_take_the_default() {
    let mapping = mapping(
        r#"{"temperature": null, "top_p": null, "stop": null, "vendor_flag": null, "vendor_name": "x"}"#,
        "{}",
    )
    .unwrap();
    assert!(mapping.standard.is_empty());
    assert_eq!(mapping.provider_fields.len(), 1);
    assert_eq!(mapping.provider_fields["vendor_name"], value(r#""x""#));
}

fn stop_accepts_the_list_form() {
    let mapping = mapping(r#"{"stop": ["a", "b"]}"#, "{}").unwrap();
    assert_eq!(mapping.standard["stop"], value(r#"["a", "b"]"#));
}

fn unsupported_names_fail_together() {
    let error = mapping(
        r#"{"reasoning_effort": "low", "metadata": {"trace": "x"}, "temperature": 0.5}"#,
        "{}",
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ParameterError::Unsupported { model, names }
            if model == "gpt-4o" && names == ["reasoning_effort".to_owned(), "metadata".to_owned()]
    ));
}

fn drop_params_removes_unsupported_names() {
    let mapping = mapping(
        r#"{"reasoning_effort": "low", "temperature": 0.5}"#,
        r#"{"drop_params": true}"#,
    )
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["reasoning_effort"]);
    assert_eq!(mapping.standard["temperature"], value("0.5"));
    assert_eq!(mapping.standard.len(), 1);
}

fn additional_drop_params_remove_supported_names() {
    let mapping = mapping(
        r#"{"temperature": 0.5, "logprobs": true}"#,
        r#"{"additional_drop_params": ["temperature"]}"#,
    )
    .unwrap();
    assert_eq!(mapping.dropped_fields.len(), 1);
    assert_eq!(mapping.dropped_fields[0], "temperature");
    assert_eq!(mapping.standard["logprobs"], value("true"));
    assert_eq!(mapping.standard.len(), 1);
}

fn additional_drop_params_remove_provider_fields() {
    let mapping = mapping(
        r#"{"vendor_flag": {"a": 1}, "vendor_name": "x"}"#,
        r#"{"additional_drop_params": ["vendor_flag"]}"#,
    )
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["vendor_flag"]);
    assert_eq!(mapping.provider_fields.len(), 1);
    assert_eq!(mapping.provider_fields["vendor_name"], value(r#""x""#));
}

fn additional_drop_params_skip_validation() {
    let mapping = mapping(
        r#"{"reasoning_effort": "low"}"#,
        r#"{"additional_drop_params": ["reasoning_effort"]}"#,
    )
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["reasoning_effort"]);
    assert!(mapping.standard.is_empty());
}

fn allowed_openai_params_extend_the_supported_list() {
    let mapping = mapping(
        r#"{"reasoning_effort": "low", "thinking": {"type": "enabled", "budget_tokens": 2048}}"#,
        r#"{"allowed_openai_params": ["reasoning_effort", "thinking"]}"#,
    )
    .unwrap();
    assert_eq!(mapping.standard["reasoning_effort"], value(r#""low""#));
    assert_eq!(
        mapping.standard["thinking"],
        value(r#"{"type": "enabled", "budget_tokens": 2048}"#)
    );
}

fn allowed_openai_params_do_not_unreserve_controls() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut parameters = parameters("{}");
    parameters
        .passthrough
        .insert("api_key".to_owned(), value(r#""sk-test""#));
    let policy = policy(r#"{"allowed_openai_params": ["api_key"]}"#);
    let error = map_parameters(&ParameterRequest {
        capabilities: query("gpt-4o", OpenAiConfigTag::Gpt, &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap_err();
    assert!(matches!(error, ParameterError::ReservedParameter { name } if name == "api_key"));
}

fn typed_passthrough_cannot_overwrite_the_typed_field() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut parameters = parameters(r#"{"temperature": 0.5}"#);
    parameters
        .passthrough
        .insert("temperature".to_owned(), value("1"));
    let policy = policy("{}");
    let error = map_parameters(&ParameterRequest {
        capabilities: query("gpt-4o", OpenAiConfigTag::Gpt, &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap_err();
    assert!(matches!(error, ParameterError::ReservedParameter { name } if name == "temperature"));
}

fn provider_fields_are_not_validated() {
    let mapping = mapping(r#"{"vendor_flag": {"a": 1}}"#, r#"{"drop_params": true}"#).unwrap();
    assert_eq!(mapping.provider_fields["vendor_flag"], value(r#"{"a": 1}"#));
    assert!(mapping.dropped_fields.is_empty());
}

fn absent_capabilities_fail_mapping() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"temperature": 0.5}"#);
    let policy = policy("{}");
    let mut query = query("claude-3", OpenAiConfigTag::Gpt, &capabilities);
    query.provider = ProviderId::Anthropic;
    let error = map_parameters(&ParameterRequest {
        capabilities: query,
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap_err();
    assert!(matches!(error, ParameterError::CapabilitiesAbsent { model } if model == "claude-3"));
}

fn typed_names_are_all_modeled_and_reserved() {
    let fields = TYPED_PARAMETER_NAMES
        .iter()
        .map(|name| format!(r#""{name}": null"#))
        .collect::<Vec<_>>()
        .join(", ");
    let parameters: ChatParameters = serde_json::from_str(&format!("{{{fields}}}")).unwrap();
    assert!(parameters.passthrough.is_empty());
    assert!(TYPED_PARAMETER_NAMES.iter().all(|name| {
        classify_parameter(name, CallType::ChatCompletion) == ParameterClass::Reserved
    }));
}

fn decimal_values_keep_their_exact_text() {
    let mapping = mapping(
        r#"{"temperature": 0.123456789012345678901234567, "top_p": 0.30000000000000000001}"#,
        "{}",
    )
    .unwrap();
    assert_eq!(
        mapping.standard["temperature"].to_string(),
        "0.123456789012345678901234567"
    );
    assert_eq!(
        mapping.standard["top_p"].to_string(),
        "0.30000000000000000001"
    );
}

fn dropped_names_are_sorted_and_unique() {
    let mapping = mapping(
        r#"{"vendor_b": 1, "vendor_a": 2, "temperature": 0.5}"#,
        r#"{"additional_drop_params": ["vendor_b", "temperature", "vendor_b"]}"#,
    )
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["temperature", "vendor_b"]);
}

fn passthrough_values_stay_raw() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut passthrough = IndexMap::new();
    passthrough.insert(
        "custom_tools".to_owned(),
        value(r#"[{"type": "function"}]"#),
    );
    let mut parameters = parameters("{}");
    parameters.passthrough = passthrough;
    let policy = policy("{}");
    let mapping = map_parameters(&ParameterRequest {
        capabilities: query("gpt-4o", OpenAiConfigTag::Gpt, &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(
        mapping.provider_fields["custom_tools"],
        value(r#"[{"type": "function"}]"#)
    );
}

fn unvalidated_user_is_dropped_not_rejected() {
    let capabilities = caps(&[]);
    let parameters = parameters(r#"{"user": "abc", "temperature": 0.5}"#);
    let policy = policy("{}");
    let mapping = map_parameters(&ParameterRequest {
        capabilities: query(
            "unknown-model",
            OpenAiConfigTag::UnknownModel,
            &capabilities,
        ),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(mapping.standard.len(), 1);
    assert_eq!(mapping.standard["temperature"], value("0.5"));
    assert_eq!(mapping.dropped_fields, ["user"]);
}

fn search_model_n_boundaries() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel, OpenAiCapability::ToolChoice]);
    let policy = policy("{}");
    let one = parameters(r#"{"n": 1}"#);
    let mapping = map_parameters(&ParameterRequest {
        capabilities: query("gpt-5-search-api", OpenAiConfigTag::Gpt5, &capabilities),
        parameters: &one,
        policy: &policy,
    })
    .unwrap();
    assert!(mapping.standard.is_empty());
    assert_eq!(mapping.dropped_fields, ["n"]);
    let two = parameters(r#"{"n": 2}"#);
    let error = map_parameters(&ParameterRequest {
        capabilities: query("gpt-5-search-api", OpenAiConfigTag::Gpt5, &capabilities),
        parameters: &two,
        policy: &policy,
    })
    .unwrap_err();
    assert!(
        matches!(error, ParameterError::Unsupported { names, .. } if names == ["n".to_owned()])
    );
}

fn extra_body_contents_become_provider_fields() {
    let mapping = mapping(r#"{"temperature": 0.5, "extra_body": {"foo": 1}}"#, "{}").unwrap();
    assert_eq!(mapping.standard.len(), 1);
    assert_eq!(mapping.standard["temperature"], value("0.5"));
    assert_eq!(mapping.provider_fields.len(), 1);
    assert_eq!(mapping.provider_fields["foo"], value("1"));
}

fn extra_body_contents_never_overwrite_top_level_fields() {
    let mapping = mapping(
        r#"{"bar": 2, "extra_body": {"bar": 1, "zeta": 1}, "zeta": 3}"#,
        "{}",
    )
    .unwrap();
    assert_eq!(mapping.provider_fields.len(), 2);
    assert_eq!(mapping.provider_fields["bar"], value("2"));
    assert_eq!(mapping.provider_fields["zeta"], value("3"));
}

fn additional_drop_params_remove_extra_body_keys() {
    let mapping = mapping(
        r#"{"extra_body": {"foo": 1, "bar": 2}}"#,
        r#"{"additional_drop_params": ["foo"]}"#,
    )
    .unwrap();
    assert_eq!(mapping.provider_fields.len(), 1);
    assert_eq!(mapping.provider_fields["bar"], value("2"));
    assert_eq!(mapping.dropped_fields, ["foo"]);
}

fn extra_body_model_key_is_dropped_and_reported() {
    let mapping = mapping(r#"{"extra_body": {"model": "x", "foo": 1}}"#, "{}").unwrap();
    assert_eq!(mapping.provider_fields.len(), 1);
    assert!(!mapping.provider_fields.contains_key("model"));
    assert_eq!(mapping.dropped_fields, ["model"]);
}

fn extra_body_typed_name_is_rejected() {
    let error = mapping(r#"{"extra_body": {"temperature": 0.2}}"#, "{}").unwrap_err();
    assert!(matches!(error, ParameterError::ReservedParameter { name } if name == "temperature"));
}

fn thinking_without_type_maps_through() {
    let mapping = mapping(
        r#"{"thinking": {"budget_tokens": 2048}}"#,
        r#"{"allowed_openai_params": ["thinking"]}"#,
    )
    .unwrap();
    assert_eq!(
        mapping.standard["thinking"],
        value(r#"{"budget_tokens": 2048}"#)
    );
}

fn extra_body_reserved_key_is_rejected() {
    let error = mapping(r#"{"extra_body": {"api_key": "sk-test"}}"#, "{}").unwrap_err();
    assert!(matches!(error, ParameterError::ReservedParameter { name } if name == "api_key"));
}

fn extra_body_whole_drop_removes_provider_fields() {
    let mapping = mapping(
        r#"{"extra_body": {"foo": 1}, "vendor_flag": 2}"#,
        r#"{"additional_drop_params": ["extra_body"]}"#,
    )
    .unwrap();
    assert!(mapping.provider_fields.is_empty());
    assert_eq!(mapping.dropped_fields, ["foo", "vendor_flag"]);
}

fn extra_body_wrong_type_is_rejected() {
    let error = mapping(r#"{"extra_body": "x"}"#, "{}").unwrap_err();
    assert!(matches!(error, ParameterError::NotAnObject { name } if name == "extra_body"));
}

fn nested_drop_paths_are_rejected() {
    let dotted = mapping(
        r#"{"tools": []}"#,
        r#"{"additional_drop_params": ["tools.function.strict"]}"#,
    )
    .unwrap_err();
    assert!(
        matches!(dotted, ParameterError::NestedDropPath { path } if path == "tools.function.strict")
    );
    let indexed = mapping(
        r#"{"tools": []}"#,
        r#"{"additional_drop_params": ["tools[0].strict"]}"#,
    )
    .unwrap_err();
    assert!(
        matches!(indexed, ParameterError::NestedDropPath { path } if path == "tools[0].strict")
    );
}

#[rstest]
#[case::supported_parameters_are_separated(supported_parameters_are_separated)]
#[case::explicit_zero_and_empty_values_survive(explicit_zero_and_empty_values_survive)]
#[case::null_values_take_the_default(null_values_take_the_default)]
#[case::stop_accepts_the_list_form(stop_accepts_the_list_form)]
#[case::unsupported_names_fail_together(unsupported_names_fail_together)]
#[case::drop_params_removes_unsupported_names(drop_params_removes_unsupported_names)]
#[case::additional_drop_params_remove_supported_names(
    additional_drop_params_remove_supported_names
)]
#[case::additional_drop_params_remove_provider_fields(
    additional_drop_params_remove_provider_fields
)]
#[case::additional_drop_params_skip_validation(additional_drop_params_skip_validation)]
#[case::allowed_openai_params_extend_the_supported_list(
    allowed_openai_params_extend_the_supported_list
)]
#[case::allowed_openai_params_do_not_unreserve_controls(
    allowed_openai_params_do_not_unreserve_controls
)]
#[case::typed_passthrough_cannot_overwrite_the_typed_field(
    typed_passthrough_cannot_overwrite_the_typed_field
)]
#[case::provider_fields_are_not_validated(provider_fields_are_not_validated)]
#[case::absent_capabilities_fail_mapping(absent_capabilities_fail_mapping)]
#[case::typed_names_are_all_modeled_and_reserved(typed_names_are_all_modeled_and_reserved)]
#[case::decimal_values_keep_their_exact_text(decimal_values_keep_their_exact_text)]
#[case::dropped_names_are_sorted_and_unique(dropped_names_are_sorted_and_unique)]
#[case::unvalidated_user_is_dropped_not_rejected(unvalidated_user_is_dropped_not_rejected)]
#[case::search_model_n_boundaries(search_model_n_boundaries)]
#[case::extra_body_contents_become_provider_fields(extra_body_contents_become_provider_fields)]
#[case::extra_body_contents_never_overwrite_top_level_fields(
    extra_body_contents_never_overwrite_top_level_fields
)]
#[case::additional_drop_params_remove_extra_body_keys(
    additional_drop_params_remove_extra_body_keys
)]
#[case::extra_body_model_key_is_dropped_and_reported(extra_body_model_key_is_dropped_and_reported)]
#[case::extra_body_reserved_key_is_rejected(extra_body_reserved_key_is_rejected)]
#[case::extra_body_typed_name_is_rejected(extra_body_typed_name_is_rejected)]
#[case::thinking_without_type_maps_through(thinking_without_type_maps_through)]
#[case::extra_body_whole_drop_removes_provider_fields(
    extra_body_whole_drop_removes_provider_fields
)]
#[case::extra_body_wrong_type_is_rejected(extra_body_wrong_type_is_rejected)]
#[case::nested_drop_paths_are_rejected(nested_drop_paths_are_rejected)]
#[case::passthrough_values_stay_raw(passthrough_values_stay_raw)]
fn provider_params_tests_behavior(#[case] scenario: fn()) {
    scenario();
}

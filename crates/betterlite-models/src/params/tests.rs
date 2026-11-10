#![cfg(test)]

use std::collections::BTreeSet;

use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Value;

use super::capabilities::{CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag};
use super::classification::{ParameterClass, classify_parameter};
use super::{
    CallType, ChatParameters, ParameterError, ParameterPolicy, ParameterRequest,
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

fn gpt_4o<'model>(
    model: &'model str,
    capabilities: &'model BTreeSet<OpenAiCapability>,
) -> CapabilityQuery<'model> {
    CapabilityQuery {
        provider: ProviderId::Openai,
        call_type: CallType::ChatCompletion,
        config: OpenAiConfigTag::Gpt,
        model: ModelProjection {
            model,
            capabilities,
        },
        base_model: None,
    }
}

fn supported_parameters_are_separated() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters =
        parameters(r#"{"temperature": 0.5, "logprobs": true, "vendor_flag": {"mode": "fast"}}"#);
    let policy = policy("{}");
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
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
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(
        r#"{"temperature": 0, "n": 0, "stop": "", "user": "", "stream": false, "logprobs": false}"#,
    );
    let policy = policy("{}");
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
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
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(
        r#"{"temperature": null, "top_p": null, "stop": null, "vendor_flag": null, "vendor_name": "x"}"#,
    );
    let policy = policy("{}");
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert!(mapping.standard.is_empty());
    assert_eq!(mapping.provider_fields.len(), 1);
    assert_eq!(mapping.provider_fields["vendor_name"], value(r#""x""#));
}

fn stop_accepts_the_list_form() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"stop": ["a", "b"]}"#);
    let policy = policy("{}");
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(mapping.standard["stop"], value(r#"["a", "b"]"#));
}

fn unsupported_names_fail_together() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(
        r#"{"reasoning_effort": "low", "metadata": {"trace": "x"}, "temperature": 0.5}"#,
    );
    let policy = policy("{}");
    let error = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap_err();
    assert!(matches!(
        error,
        ParameterError::Unsupported { model, names }
            if model == "gpt-4o" && names == ["reasoning_effort".to_owned(), "metadata".to_owned()]
    ));
}

fn drop_params_removes_unsupported_names() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"reasoning_effort": "low", "temperature": 0.5}"#);
    let policy = policy(r#"{"drop_params": true}"#);
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["reasoning_effort"]);
    assert_eq!(mapping.standard["temperature"], value("0.5"));
    assert_eq!(mapping.standard.len(), 1);
}

fn additional_drop_params_remove_supported_names() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"temperature": 0.5, "logprobs": true}"#);
    let policy = policy(r#"{"additional_drop_params": ["temperature"]}"#);
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["temperature"]);
    assert_eq!(mapping.standard["logprobs"], value("true"));
    assert_eq!(mapping.standard.len(), 1);
}

fn additional_drop_params_remove_provider_fields() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"vendor_flag": {"a": 1}, "vendor_name": "x"}"#);
    let policy = policy(r#"{"additional_drop_params": ["vendor_flag"]}"#);
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["vendor_flag"]);
    assert_eq!(mapping.provider_fields.len(), 1);
    assert_eq!(mapping.provider_fields["vendor_name"], value(r#""x""#));
}

fn additional_drop_params_skip_validation() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"reasoning_effort": "low"}"#);
    let policy = policy(r#"{"additional_drop_params": ["reasoning_effort"]}"#);
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["reasoning_effort"]);
    assert!(mapping.standard.is_empty());
}

fn allowed_openai_params_extend_the_supported_list() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(
        r#"{"reasoning_effort": "low", "thinking": {"type": "enabled", "budget_tokens": 2048}}"#,
    );
    let policy = policy(r#"{"allowed_openai_params": ["reasoning_effort", "thinking"]}"#);
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
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
        capabilities: gpt_4o("gpt-4o", &capabilities),
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
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap_err();
    assert!(matches!(error, ParameterError::ReservedParameter { name } if name == "temperature"));
}

fn provider_fields_are_not_validated() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"vendor_flag": {"a": 1}}"#);
    let policy = policy(r#"{"drop_params": true}"#);
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(mapping.provider_fields["vendor_flag"], value(r#"{"a": 1}"#));
    assert!(mapping.dropped_fields.is_empty());
}

fn absent_capabilities_fail_mapping() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"temperature": 0.5}"#);
    let policy = policy("{}");
    let mut query = gpt_4o("claude-3", &capabilities);
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
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"temperature": 0.1, "top_p": 2.5, "presence_penalty": -1.25}"#);
    let policy = policy("{}");
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(mapping.standard["temperature"].to_string(), "0.1");
    assert_eq!(mapping.standard["top_p"].to_string(), "2.5");
    assert_eq!(mapping.standard["presence_penalty"].to_string(), "-1.25");
}

fn dropped_names_are_sorted_and_unique() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let parameters = parameters(r#"{"vendor_b": 1, "vendor_a": 2, "temperature": 0.5}"#);
    let policy = policy(r#"{"additional_drop_params": ["vendor_b", "temperature", "vendor_b"]}"#);
    let mapping = map_parameters(&ParameterRequest {
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
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
        capabilities: gpt_4o("gpt-4o", &capabilities),
        parameters: &parameters,
        policy: &policy,
    })
    .unwrap();
    assert_eq!(
        mapping.provider_fields["custom_tools"],
        value(r#"[{"type": "function"}]"#)
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
#[case::passthrough_values_stay_raw(passthrough_values_stay_raw)]
fn provider_params_tests_behavior(#[case] scenario: fn()) {
    scenario();
}

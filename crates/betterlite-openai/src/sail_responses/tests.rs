#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "json! fixtures state the sail wire the transforms read, numbers included, and each test keeps to one scenario"
    )
)]

use std::future::{Future, ready};

use betterlite_error::error::ErrorClass;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};

use super::error::SailResponsesError;
use super::{MapRequest, SailResponsesConfig};
use crate::json_provider_policy::responses::ResponsesPolicy;
use crate::reasoning_models::{DropParams, ReasoningCapabilities};
use crate::response_input::ResponsesInput;
use crate::response_params::bundled::BundledReasoningModels;
use crate::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use crate::response_params::params::ResponsesParams;
use crate::sail_common::SailSettings;

const MODEL: &str = "llama-3-70b";

struct NoCapabilities;

impl ReasoningCapabilities for NoCapabilities {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn catalogue_declares_default(&mut self) -> bool {
        false
    }
}

impl ResponsesCapabilities for NoCapabilities {
    fn model_info(&mut self, _model: &str) -> impl Future<Output = ModelInfo> + Send {
        ready(ModelInfo::Unavailable)
    }
}

fn config(global_drop_params: bool) -> SailResponsesConfig {
    let registry = JSONProviderRegistry::load();
    let descriptor = registry.get("sail").unwrap().clone();
    SailResponsesConfig {
        policy: ResponsesPolicy { descriptor },
        settings: SailSettings { global_drop_params },
    }
}

fn object(value: Value) -> Object {
    serde_json::from_value(value).unwrap()
}

fn responses_params(value: Value) -> ResponsesParams {
    serde_json::from_value(value).unwrap()
}

async fn mapped(
    params: Value,
    drop_params: DropParams,
    global_drop_params: bool,
) -> Result<Value, SailResponsesError> {
    let mut capabilities = NoCapabilities;
    let mut bundled = BundledReasoningModels::default();
    let parsed = responses_params(params);
    let object = config(global_drop_params)
        .map_openai_params(MapRequest {
            capabilities: &mut capabilities,
            model: MODEL,
            params: &parsed,
            drop_params,
            bundled: &mut bundled,
        })
        .await?;
    Ok(serde_json::to_value(object).unwrap())
}

fn input() -> ResponsesInput {
    ResponsesInput::Text(String::from("hi"))
}

#[tokio::test]
async fn the_mapping_clamps_max_output_tokens_through_the_parent() {
    let result = mapped(
        json!({"max_output_tokens": 8}),
        DropParams::default(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(result, json!({"max_output_tokens": 16}));
}

#[tokio::test]
async fn the_tier_becomes_the_metadata_window() {
    let result = mapped(
        json!({"service_tier": "default", "user": "u-1"}),
        DropParams::default(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        result,
        json!({"user": "u-1", "metadata": {"completion_window": "asap"}})
    );
}

#[tokio::test]
async fn a_conflicting_tier_and_caller_window_fail_with_the_sail_class() {
    let error = mapped(
        json!({"service_tier": "default", "metadata": {"completion_window": "flex"}}),
        DropParams::default(),
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), 400);
    let Some(ErrorClass::UnsupportedParams(context)) = error.class() else {
        panic!("a sail conflict is an unsupported params error");
    };
    assert_eq!(context.provider.as_deref(), Some("sail"));
    assert_eq!(context.model.as_deref(), Some(MODEL));
}

#[tokio::test]
async fn dropping_through_the_request_flag_or_the_global_setting_removes_a_bad_tier() {
    let by_request = mapped(
        json!({"service_tier": "flash"}),
        DropParams {
            global: false,
            request: true,
        },
        false,
    )
    .await
    .unwrap();
    assert_eq!(by_request, json!({}));
    let by_global = mapped(
        json!({"service_tier": "flash"}),
        DropParams::default(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(by_global, json!({}));
}

#[tokio::test]
async fn a_null_metadata_is_dropped_and_an_empty_one_is_kept() {
    let null = mapped(json!({"metadata": null}), DropParams::default(), false)
        .await
        .unwrap();
    assert_eq!(null, json!({}));
    let empty = mapped(json!({"metadata": {}}), DropParams::default(), false)
        .await
        .unwrap();
    assert_eq!(empty, json!({"metadata": {}}));
}

#[tokio::test]
async fn a_null_tier_is_removed_without_a_window() {
    let result = mapped(json!({"service_tier": null}), DropParams::default(), false)
        .await
        .unwrap();
    assert_eq!(result, json!({}));
}

#[test]
fn the_request_body_drops_the_service_tier_and_keeps_the_rest() {
    let params = object(json!({"service_tier": "flex", "temperature": 0.2, "store": true}));
    let body = config(false)
        .transform_request(MODEL, &input(), &params, None)
        .unwrap();
    let value = serde_json::to_value(&body).unwrap();
    assert_eq!(value.get("service_tier"), None);
    assert_eq!(value.get("model"), Some(&json!(MODEL)));
    assert_eq!(value.get("input"), Some(&json!("hi")));
    assert_eq!(value.get("store"), Some(&json!(true)));
    assert_eq!(value.get("temperature"), Some(&json!(0.2)));
}

#[test]
fn a_duplicate_body_key_maps_to_a_server_error() {
    let params = object(json!({"model": "m"}));
    let error = config(false)
        .transform_request(MODEL, &input(), &params, None)
        .unwrap_err();
    assert_eq!(error.status_code(), 500);
    assert!(error.class().is_none());
}

#[test]
fn a_non_list_tools_value_keeps_its_bad_request_status() {
    let params = object(json!({"tools": "raw"}));
    let error = config(false)
        .transform_request(MODEL, &input(), &params, None)
        .unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert!(error.class().is_none());
}

#[test]
fn transform_extra_body_rejects_a_tier_and_a_caller_window() {
    let request = object(json!({"metadata": {"completion_window": "flex"}}));
    let tier = config(false)
        .transform_extra_body(
            &object(json!({"service_tier": "flex"})),
            &request,
            MODEL,
            DropParams::default(),
        )
        .unwrap_err();
    assert_eq!(tier.status_code(), 400);
    let window = config(false)
        .transform_extra_body(
            &object(json!({"metadata": {"completion_window": "asap"}})),
            &request,
            MODEL,
            DropParams::default(),
        )
        .unwrap_err();
    assert_eq!(window.status_code(), 400);
}

#[test]
fn transform_extra_body_merges_the_request_metadata_over_the_caller_one() {
    let request = object(json!({"metadata": {"completion_window": "balanced"}}));
    let merged = config(false)
        .transform_extra_body(
            &object(json!({"user": "u", "metadata": {"trace": "t-1"}})),
            &request,
            MODEL,
            DropParams::default(),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&merged).unwrap(),
        json!({"user": "u", "metadata": {"trace": "t-1", "completion_window": "balanced"}})
    );
}

#[test]
fn transform_extra_body_drops_a_tier_through_the_request_or_the_global_flag() {
    let request = object(json!({}));
    let extra = object(json!({"service_tier": "flex", "user": "u"}));
    let by_request = config(false)
        .transform_extra_body(
            &extra,
            &request,
            MODEL,
            DropParams {
                global: false,
                request: true,
            },
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&by_request).unwrap(),
        json!({"user": "u"})
    );
    let by_global = config(true)
        .transform_extra_body(&extra, &request, MODEL, DropParams::default())
        .unwrap();
    assert_eq!(
        serde_json::to_value(&by_global).unwrap(),
        json!({"user": "u"})
    );
}

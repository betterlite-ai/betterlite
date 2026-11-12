use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::request_pricing::custom_fields::CUSTOM_PRICING_FIELDS;
use betterlite_models::request_pricing::entry::{build_custom_pricing_entry, router_deployment_id};
use betterlite_models::request_pricing::fields::{
    COST_MAP_LOOKUP_KEY, DEPLOYMENT_SCOPED_FIELDS, PRICING_FIELD_EXEMPTIONS, PRICING_OVERRIDES_KEY,
    echoed_cost_map_fields, echoed_cost_map_pricing_fields, is_server_derived_pricing_key,
    pricing_override_fields, shared_backend_model_info, strip_custom_pricing_fields,
    without_server_derived_pricing,
};
use betterlite_models::request_pricing::shared_fields::SHARED_MODEL_INFO_FIELDS;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::completion_options::control::resolve_control_options;
use betterlite_sdk::completion_options::error::CompletionOptionsError;
use betterlite_sdk::completion_options::tool_keys::{
    drop_anthropic_only_tool_keys, is_claude_tool_target,
};
use project::project;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

mod project;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Input {
    Options(OptionsCall),
    Pricing(PricingCall),
    Fields(FieldsCall),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum OptionsCall {
    Control {
        kwargs: RequestMap,
        model: String,
        global_drop_params: Option<bool>,
    },
    ClaudeTarget {
        custom_llm_provider: Option<String>,
        model: String,
    },
    ToolKeys {
        tools: Option<Vec<Json>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PricingCall {
    PricingEntry {
        custom_llm_provider: String,
        kwargs: JsonObject,
        model_info: Option<JsonObject>,
    },
    DeploymentId {
        kwargs: JsonObject,
    },
    CustomFields,
    SharedFields,
    Constants,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum FieldsCall {
    Strip {
        model_info: JsonObject,
    },
    SharedInfo {
        model_info: JsonObject,
    },
    ServerDerivedKey {
        key: String,
    },
    WithoutServerDerived {
        model_info: JsonObject,
    },
    EchoedPricing {
        model_info: JsonObject,
    },
    EchoedFields {
        model_info: JsonObject,
        entries: Vec<JsonObject>,
    },
    OverrideFields {
        sources: Vec<JsonObject>,
    },
}

fn sorted_names(names: &[&str]) -> Value {
    let mut sorted = names.to_vec();
    sorted.sort_unstable();
    json!(sorted)
}

fn control(kwargs: &RequestMap, model: &str, global_drop_params: Option<bool>) -> HelperResult {
    match resolve_control_options(kwargs, model, global_drop_params) {
        Ok(control_options) => Ok(json!({"stream_chunk_size": control_options.stream_chunk_size})),
        Err(CompletionOptionsError::InvalidControlOption(details)) => Err(Box::new(HelperError {
            class: "BadRequestError".to_owned(),
            status: Some(details.status.0),
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: details.message,
        })),
    }
}

fn options(call: &OptionsCall) -> HelperResult {
    match call {
        OptionsCall::Control {
            kwargs,
            model,
            global_drop_params,
        } => control(kwargs, model, *global_drop_params),
        OptionsCall::ClaudeTarget {
            custom_llm_provider,
            model,
        } => Ok(json!(is_claude_tool_target(
            custom_llm_provider.as_deref(),
            model
        ))),
        OptionsCall::ToolKeys { tools } => {
            let kept = drop_anthropic_only_tool_keys(tools.as_deref());
            let shown = kept.map_or(Ok(Value::Null), |list| project(&Json::List(list)))?;
            Ok(json!({"tools": shown}))
        }
    }
}

fn pricing(call: &PricingCall) -> HelperResult {
    match call {
        PricingCall::PricingEntry {
            custom_llm_provider,
            kwargs,
            model_info,
        } => project(&Json::Object(build_custom_pricing_entry(
            custom_llm_provider,
            kwargs,
            model_info.as_ref(),
        ))),
        PricingCall::DeploymentId { kwargs } => {
            Ok(json!(router_deployment_id(kwargs).or_value_error()?))
        }
        PricingCall::CustomFields => Ok(sorted_names(CUSTOM_PRICING_FIELDS)),
        PricingCall::SharedFields => Ok(sorted_names(SHARED_MODEL_INFO_FIELDS)),
        PricingCall::Constants => Ok(json!({
            "deployment_scoped": sorted_names(DEPLOYMENT_SCOPED_FIELDS),
            "exemptions": sorted_names(PRICING_FIELD_EXEMPTIONS),
            "overrides_key": PRICING_OVERRIDES_KEY,
            "lookup_key": COST_MAP_LOOKUP_KEY,
        })),
    }
}

fn fields(call: &FieldsCall) -> HelperResult {
    match call {
        FieldsCall::Strip { model_info } => {
            project(&Json::Object(strip_custom_pricing_fields(model_info)))
        }
        FieldsCall::SharedInfo { model_info } => {
            project(&Json::Object(shared_backend_model_info(model_info)))
        }
        FieldsCall::ServerDerivedKey { key } => Ok(json!(is_server_derived_pricing_key(key))),
        FieldsCall::WithoutServerDerived { model_info } => {
            project(&Json::Object(without_server_derived_pricing(model_info)))
        }
        FieldsCall::EchoedPricing { model_info } => {
            Ok(json!(echoed_cost_map_pricing_fields(model_info)))
        }
        FieldsCall::EchoedFields {
            model_info,
            entries,
        } => {
            let found: Vec<&JsonObject> = entries.iter().collect();
            Ok(json!(echoed_cost_map_fields(model_info, &found)))
        }
        FieldsCall::OverrideFields { sources } => {
            let found: Vec<&JsonObject> = sources.iter().collect();
            Ok(json!(pricing_override_fields(&found)))
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Options(call) => options(call),
            Input::Pricing(call) => pricing(call),
            Input::Fields(call) => fields(call),
        }
    }
}

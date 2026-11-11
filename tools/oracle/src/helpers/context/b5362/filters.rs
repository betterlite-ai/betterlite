use std::convert::Infallible;

use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::filters::{
    apply_openai_param_overrides, bedrock_route_for_request, filter_out_litellm_params,
    get_non_default_params, provider_rejectable_params, remove_unsupported_params,
    select_non_default_params,
};
use betterlite_context::non_default_params::names::{
    OPENAI_CHAT_COMPLETION_PARAMS, OPENAI_TRANSCRIPTION_PARAMS,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::Failure;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Endpoint {
    Completion,
    Transcription,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum FilterCall {
    SelectNonDefault {
        passed_params: Params,
        default_names: Vec<String>,
        additional_drop_params: Option<Vec<String>>,
    },
    GetNonDefaultParams {
        passed_params: Params,
    },
    ProviderRejectable {
        passed_params: Params,
    },
    FilterOutLitellm {
        kwargs: Params,
        excluding: Vec<String>,
    },
    EndpointKwargs {
        kwargs: Params,
        endpoint: Endpoint,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum MappingCall {
    RemoveUnsupported {
        non_default_params: Params,
        supported: Option<Vec<String>>,
    },
    ApplyOverrides {
        optional_params: Params,
        non_default_params: Params,
        allowed: Vec<String>,
    },
    BedrockRoute {
        model: String,
        passed_params: Params,
        additional_drop_params: Option<Vec<String>>,
    },
}

pub(super) fn run(call: &FilterCall) -> Result<Value, Failure> {
    let value = match call {
        FilterCall::SelectNonDefault {
            passed_params,
            default_names,
            additional_drop_params,
        } => {
            let names: Vec<&str> = default_names.iter().map(String::as_str).collect();
            let drops = additional_drop_params.as_deref().unwrap_or_default();
            serde_json::to_value(select_non_default_params(passed_params, &names, drops))?
        }
        FilterCall::GetNonDefaultParams { passed_params } => {
            serde_json::to_value(get_non_default_params(passed_params))?
        }
        FilterCall::ProviderRejectable { passed_params } => {
            serde_json::to_value(provider_rejectable_params(passed_params))?
        }
        FilterCall::FilterOutLitellm { kwargs, excluding } => {
            let names: Vec<&str> = excluding.iter().map(String::as_str).collect();
            serde_json::to_value(filter_out_litellm_params(kwargs, &names))?
        }
        FilterCall::EndpointKwargs { kwargs, endpoint } => {
            let names = match endpoint {
                Endpoint::Completion => OPENAI_CHAT_COMPLETION_PARAMS,
                Endpoint::Transcription => OPENAI_TRANSCRIPTION_PARAMS,
            };
            serde_json::to_value(filter_out_litellm_params(kwargs, names))?
        }
    };
    Ok(value)
}

pub(super) fn run_mapping(call: &MappingCall) -> Result<Value, Failure> {
    let value = match call {
        MappingCall::RemoveUnsupported {
            non_default_params,
            supported,
        } => serde_json::to_value(remove_unsupported_params(
            non_default_params.clone(),
            supported.as_deref(),
        ))?,
        MappingCall::ApplyOverrides {
            optional_params,
            non_default_params,
            allowed,
        } => {
            let mut remaining = non_default_params.clone();
            let optional =
                apply_openai_param_overrides(optional_params.clone(), &mut remaining, allowed);
            json!({ "optional_params": optional, "non_default_params": remaining })
        }
        MappingCall::BedrockRoute {
            model,
            passed_params,
            additional_drop_params,
        } => {
            let drops = additional_drop_params.as_deref().unwrap_or_default();
            let (routed, filtered) =
                bedrock_route_for_request(model, passed_params, drops, |name, kept| {
                    Ok::<_, Infallible>((name.to_owned(), kept.clone()))
                })
                .unwrap_or_else(|never| match never {});
            json!({ "model": routed, "params": filtered })
        }
    };
    Ok(value)
}

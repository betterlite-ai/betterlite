use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::prompt_dispatch::identity::resolve_prompt_identity;
use betterlite_router::prompt_dispatch::known_callbacks::KNOWN_CALLBACKS;
use betterlite_router::prompt_dispatch::merge::{PromptCall, merge_prompt_request};
use betterlite_router::prompt_dispatch::ports::CompiledPrompt;
use betterlite_router::prompt_dispatch::predicate::is_prompt_model;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    IsPromptModel {
        model: String,
        #[serde(default)]
        model_list: Vec<DeploymentRow>,
    },
    ResolvePromptIdentity {
        request: JsonObject,
        deployment: DeploymentRow,
    },
    MergePromptRequest {
        request: JsonObject,
        deployment: DeploymentRow,
        compiled: CompiledRow,
        new_model_has_deployments: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentRow {
    model_name: String,
    #[serde(default)]
    litellm_params: JsonObject,
    #[serde(default)]
    model_info: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompiledRow {
    model: String,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    optional_params: JsonObject,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::IsPromptModel { model, model_list } => {
                let deployments: Vec<DeploymentSpec> = model_list
                    .iter()
                    .filter(|row| row.model_name == *model)
                    .map(|row| DeploymentSpec {
                        model_name: row.model_name.clone(),
                        litellm_params: row.litellm_params.clone(),
                        model_info: row.model_info.clone(),
                        extra: JsonObject::new(),
                    })
                    .collect();
                Ok(json!({"is_prompt_model": is_prompt_model(&deployments, &KNOWN_CALLBACKS)}))
            }
            Input::ResolvePromptIdentity {
                request,
                deployment,
            } => {
                let identity = resolve_prompt_identity(request, &deployment.litellm_params)
                    .map_err(|error| value_error(error.to_string()))?;
                Ok(json!({
                    "prompt_id": identity.prompt_id,
                    "prompt_variables": identity.prompt_variables,
                    "prompt_label": identity.prompt_label,
                }))
            }
            Input::MergePromptRequest {
                request,
                deployment,
                compiled,
                new_model_has_deployments,
            } => {
                let identity = resolve_prompt_identity(request, &deployment.litellm_params)
                    .map_err(|error| value_error(error.to_string()))?;
                let compiled = CompiledPrompt {
                    model: compiled.model.clone(),
                    messages: compiled.messages.clone(),
                    optional_params: compiled.optional_params.clone(),
                };
                let merged = merge_prompt_request(
                    &deployment.litellm_params,
                    request,
                    &compiled,
                    &identity,
                    *new_model_has_deployments,
                );
                let call = match merged.call {
                    PromptCall::Direct => "direct",
                    PromptCall::WithFallbacks => "with_fallbacks",
                };
                Ok(json!({"request": merged.fields, "call": call}))
            }
        }
    }
}

fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

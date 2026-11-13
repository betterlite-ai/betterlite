use std::error::Error;

use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::filters::{Endpoint, non_default_kwargs};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use crate::prompt_dispatch::identity::{PromptIdentityError, resolve_prompt_identity};
use crate::prompt_dispatch::merge::{PromptCall, merge_prompt_request};
use crate::prompt_dispatch::ports::{CompilePromptCall, PromptPorts};

#[derive(Debug, Error)]
pub enum PromptFactoryError<PortError: Error + 'static> {
    #[error(transparent)]
    Identity(#[from] PromptIdentityError),
    #[error("the prompt management deployment has no model in litellm_params")]
    DeploymentModel,
    #[error(transparent)]
    Port(PortError),
}

// Router._prompt_management_factory: the whole call goes to the prompt backend, then to the deployment or the fallback loop it names.
pub async fn prompt_factory<Ports>(
    model: &str,
    messages: &[Json],
    mut request: JsonObject,
    ports: &Ports,
) -> Result<Ports::Response, PromptFactoryError<Ports::Error>>
where
    Ports: PromptPorts + ?Sized,
{
    let specific_deployment = request.shift_remove("specific_deployment");
    let deployment = ports
        .select_deployment(model, messages, specific_deployment.as_ref(), &request)
        .await
        .map_err(PromptFactoryError::Port)?;
    ports.update_with_deployment(&mut request, &deployment);
    let data = deployment.litellm_params;
    let identity = resolve_prompt_identity(&request, &data)?;
    let Some(Json::Text(deployment_model)) = data.get("model") else {
        return Err(PromptFactoryError::DeploymentModel);
    };
    let non_default = completion_non_default_params(&request);
    let compiled = ports
        .compile_prompt(CompilePromptCall {
            model: deployment_model.as_str(),
            messages,
            non_default_params: &non_default,
            prompt_id: identity.prompt_id.as_ref(),
            prompt_variables: identity.prompt_variables.as_ref(),
            prompt_label: identity.prompt_label.as_ref(),
            request: &request,
            injected_for_every_deployment: true,
        })
        .await
        .map_err(PromptFactoryError::Port)?;
    let merged = merge_prompt_request(
        &data,
        &request,
        &compiled,
        &identity,
        ports.has_deployments(&compiled.model),
    );
    match merged.call {
        PromptCall::Direct => ports
            .call_direct(merged.fields)
            .await
            .map_err(PromptFactoryError::Port),
        PromptCall::WithFallbacks => ports
            .call_with_fallbacks(merged.fields)
            .await
            .map_err(PromptFactoryError::Port),
    }
}

// litellm.utils.get_non_default_completion_params: every key of the request that is neither an OpenAI chat param nor litellm's own.
fn completion_non_default_params(request: &JsonObject) -> JsonObject {
    let params: Params = request
        .iter()
        .map(|(key, value)| (key.clone(), param_from(value)))
        .collect();
    non_default_kwargs(&params, Endpoint::Completion)
        .into_iter()
        .map(|(key, value)| (key, json_from_param(&value)))
        .collect()
}

fn param_from(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(param_from).collect()),
        Json::Object(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), param_from(item)))
                .collect(),
        ),
    }
}

fn json_from_param(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::Text(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(json_from_param).collect()),
        JsonValue::Object(entries) => Json::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), json_from_param(item)))
                .collect(),
        ),
    }
}

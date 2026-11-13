use super::messages::{optional, text_of};
use crate::completion_request::Preparation;
use crate::completion_request::args::{CompletionArgs, CompletionSettings, JsonObject};
use crate::completion_request::call::{
    FallbackCall, ModelListCall, PreparedCompletion, PreparedTimeout,
};
use crate::completion_request::error::{CompletionRequestError, RequestResult};
use crate::completion_request::ports::{ProviderLookup, TimeoutRequest};
use betterlite_models::decisions::Json;
use betterlite_models::request_pricing::register::CustomPricingRequest;
pub(crate) fn fallbacks_for(
    args: &CompletionArgs,
    settings: &CompletionSettings,
) -> RequestResult<Option<Vec<Json>>> {
    match args.kwargs.get("fallbacks") {
        None | Some(Json::Null) => Ok(settings.model_fallbacks.clone()),
        Some(Json::List(items)) => Ok(Some(items.clone())),
        Some(_) => Err(CompletionRequestError::FieldType {
            field: "fallbacks",
            expected: "a list",
        }),
    }
}
fn model_deployments(state: &Preparation<'_>) -> RequestResult<Vec<JsonObject>> {
    let Some(model_list) = &state.args.model_list else {
        return Ok(Vec::new());
    };
    let mut deployments = Vec::new();
    for entry in model_list {
        let name = entry.get("model_name").and_then(text_of).ok_or(
            CompletionRequestError::ModelListEntry {
                field: "model_name",
            },
        )?;
        if name != state.call.model {
            continue;
        }
        match entry.get("litellm_params") {
            Some(Json::Object(params)) => deployments.push(params.clone()),
            None
            | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(CompletionRequestError::ModelListEntry {
                    field: "litellm_params",
                });
            }
        }
    }
    Ok(deployments)
}
pub(crate) fn early_exit(state: &mut Preparation<'_>) -> RequestResult<Option<PreparedCompletion>> {
    state.snapshot.kwargs.clone_from(&state.kwargs);
    let args = state.snapshot.clone();
    if let Some(fallbacks) = fallbacks_for(state.args, state.settings)? {
        return Ok(Some(PreparedCompletion::Fallbacks(FallbackCall {
            fallbacks,
            args,
        })));
    }
    if state.args.model_list.is_none() {
        return Ok(None);
    }
    let deployments = model_deployments(state)?;
    Ok(Some(PreparedCompletion::ModelList(ModelListCall {
        deployments,
        args,
    })))
}
fn base_model(state: &Preparation<'_>) -> RequestResult<Option<String>> {
    if let Some(base) = optional::<String>(&state.kwargs, "base_model")? {
        return Ok(Some(base));
    }
    let info = optional::<JsonObject>(&state.kwargs, "model_info")?;
    Ok(info
        .as_ref()
        .and_then(|model_info| model_info.get("base_model"))
        .and_then(text_of)
        .map(str::to_owned))
}
pub(crate) fn resolve(state: &mut Preparation<'_>) -> RequestResult<String> {
    state.call.base_model = base_model(state)?;
    if let Some(alias) = state.settings.model_alias_map.get(&state.call.model) {
        state.call.model = alias.clone();
    }
    if optional::<bool>(&state.kwargs, "azure")?.unwrap_or(false) {
        state.call.custom_llm_provider = Some(String::from("azure"));
    }
    if let Some(deployment) = &state.args.deployment_id {
        state.call.model = deployment.clone();
        state.call.custom_llm_provider = Some(String::from("azure"));
    }
    let lookup = ProviderLookup {
        model: &state.call.model,
        custom_llm_provider: state.call.custom_llm_provider.as_deref(),
        api_base: state.call.api_base.as_deref(),
        api_key: state.args.api_key.as_ref(),
    };
    let resolved = state.ports.model_metadata.resolve_provider(&lookup)?;
    let provider = resolved.provider.slug().to_owned();
    state.call.model = resolved.model;
    state.call.custom_llm_provider = Some(provider.clone());
    state.call.api_base = resolved
        .api_base
        .clone()
        .or_else(|| state.call.api_base.clone());
    state.call.api_key = resolved.api_key.or_else(|| state.args.api_key.clone());
    Ok(provider)
}
pub(crate) fn hidden_params(state: &mut Preparation<'_>, provider: &str) {
    let region = state
        .kwargs
        .get("aws_region_name")
        .cloned()
        .unwrap_or(Json::Null);
    state.call.hidden_params.insert(
        String::from("custom_llm_provider"),
        Json::Text(provider.to_owned()),
    );
    state
        .call
        .hidden_params
        .insert(String::from("region_name"), region);
}
pub(crate) fn timeout(state: &Preparation<'_>, provider: &str) -> RequestResult<PreparedTimeout> {
    state.ports.timeout.resolve(&TimeoutRequest {
        model_timeout: state.args.timeout.as_ref(),
        timeout: state.kwargs.get("timeout"),
        request_timeout: state.kwargs.get("request_timeout"),
        configured_request_timeout: state.settings.request_timeout,
        custom_llm_provider: provider,
    })
}
fn pricing_gate(kwargs: &JsonObject) -> bool {
    let given =
        |name: &str| matches!(kwargs.get(name), Some(value) if !matches!(value, Json::Null));
    (given("input_cost_per_token") && given("output_cost_per_token"))
        || given("input_cost_per_second")
        || given("output_cost_per_second")
        || given("cost_per_second")
}
pub(crate) async fn pricing(state: &Preparation<'_>, provider: &str) -> RequestResult<()> {
    if !pricing_gate(&state.kwargs) {
        return Ok(());
    }
    let model_info = optional::<JsonObject>(&state.kwargs, "model_info")?;
    let request = CustomPricingRequest {
        model: &state.call.model,
        custom_llm_provider: provider,
        kwargs: &state.kwargs,
        model_info: model_info.as_ref(),
    };
    state
        .ports
        .pricing
        .register_custom_pricing(&request)
        .await?;
    Ok(())
}
pub(crate) fn custom_prompt(state: &mut Preparation<'_>) {
    let template: JsonObject = [
        "initial_prompt_value",
        "roles",
        "final_prompt_value",
        "bos_token",
        "eos_token",
    ]
    .iter()
    .filter_map(|name| {
        let value = state
            .kwargs
            .get(*name)
            .filter(|value| !matches!(value, Json::Null))?;
        Some(((*name).to_owned(), value.clone()))
    })
    .collect();
    if template.is_empty() {
        return;
    }
    state.call.custom_prompt_dict =
        JsonObject::from([(state.call.model.clone(), Json::Object(template))]);
}

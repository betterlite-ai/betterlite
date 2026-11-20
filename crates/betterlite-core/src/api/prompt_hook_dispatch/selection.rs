use std::error::Error;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::anthropic_cache_callbacks::gate::cache_control_custom_logger;

use super::json_bridge::prompt_object;
use super::{PromptHookDispatch, PromptManager};

const INTEGRATION_KEY: &str = "prompt_integration";
const VECTOR_STORE_INTEGRATION: &str = "vector_store_pre_call_hook";

pub struct PromptSelectionRequest<'request> {
    pub model: &'request str,
    pub non_default_params: &'request JsonObject,
    pub tools: Option<&'request [JsonObject]>,
    pub prompt_id: Option<&'request str>,
    pub prompt_spec: Option<&'request PromptSpec>,
    pub dynamic_callback_params: Option<&'request StandardCallbackDynamicParams>,
}

// LiteLLM's get_custom_logger_for_prompt_management; the six steps run first match wins.
pub fn select_prompt_manager<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: &PromptSelectionRequest<'_>,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>
where
    BackendError: Error + Send + Sync + 'static,
{
    if let Some(manager) = compatible_callback_manager(dispatch, request.model) {
        return Some(manager);
    }
    if let Some(manager) = auto_detected_manager(dispatch, request) {
        return Some(manager);
    }
    if let Some(manager) = fallback_manager(dispatch, request) {
        return Some(manager);
    }
    if let Some(manager) = cache_control_manager(dispatch, request.non_default_params) {
        return Some(manager);
    }
    vector_store_manager(dispatch)
}

// Step 1: a known compatible callback name the model starts with wins, and the stamp is the model prefix.
fn compatible_callback_manager<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    model: &str,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>
where
    BackendError: Error + Send + Sync + 'static,
{
    let known = dispatch.factory.known_callbacks();
    for name in known {
        if !model.starts_with(*name) {
            continue;
        }
        let Some(manager) = dispatch.factory.init(name) else {
            continue;
        };
        let integration = model.split_once('/').map_or(model, |(head, _)| head);
        dispatch.state.model_call_details.insert(
            INTEGRATION_KEY.to_owned(),
            Json::Text(integration.to_owned()),
        );
        return Some(manager);
    }
    None
}

// Step 2: with a prompt id and dynamic params, the first manager whose own check says yes owns the prompt.
fn auto_detected_manager<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: &PromptSelectionRequest<'_>,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>
where
    BackendError: Error + Send + Sync + 'static,
{
    let prompt_id = request.prompt_id.filter(|id| !id.is_empty())?;
    let dynamic = request.dynamic_callback_params?;
    let found = dispatch
        .managers
        .iter()
        .find(|manager| {
            manager
                .should_run(Some(prompt_id), request.prompt_spec, dynamic)
                .unwrap_or(false)
        })
        .map(Arc::clone)?;
    stamp_manager(dispatch, found.as_ref());
    Some(found)
}

// Step 3: without a prompt id a manager must run without one; with a prompt id the first manager wins unchecked.
fn fallback_manager<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: &PromptSelectionRequest<'_>,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>
where
    BackendError: Error + Send + Sync + 'static,
{
    let found = if request.prompt_id.is_some() {
        dispatch.managers.first().map(Arc::clone)
    } else {
        let defaults = Json::Object(JsonObject::new());
        let dynamic = request.dynamic_callback_params.unwrap_or(&defaults);
        dispatch
            .managers
            .iter()
            .find(|manager| {
                manager
                    .should_run(None, request.prompt_spec, dynamic)
                    .unwrap_or(false)
            })
            .map(Arc::clone)
    };
    let manager = found?;
    stamp_manager(dispatch, manager.as_ref());
    Some(manager)
}

// Step 4: B4126's selector, which needs a truthy cache_control_injection_points.
fn cache_control_manager<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    non_default_params: &JsonObject,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>
where
    BackendError: Error + Send + Sync + 'static,
{
    let params = prompt_object(non_default_params);
    let manager = cache_control_custom_logger(&params, |name| dispatch.factory.init(name))?;
    stamp_manager(dispatch, manager.as_ref());
    Some(manager)
}

// Step 5: a configured registry ends the search, and its hook joins the global callback list once.
fn vector_store_manager<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>
where
    BackendError: Error + Send + Sync + 'static,
{
    dispatch.vector_stores?;
    let manager = dispatch.factory.init(VECTOR_STORE_INTEGRATION)?;
    stamp_manager(dispatch, manager.as_ref());
    if !dispatch
        .callbacks
        .iter()
        .any(|existing| Arc::ptr_eq(existing, &manager))
    {
        dispatch.callbacks.push(Arc::clone(&manager));
    }
    Some(manager)
}

fn stamp_manager<BackendError>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    manager: &dyn PromptManager<BackendError>,
) where
    BackendError: Error + Send + Sync + 'static,
{
    dispatch.state.model_call_details.insert(
        INTEGRATION_KEY.to_owned(),
        Json::Text(manager.integration_name().to_owned()),
    );
}

mod tests;

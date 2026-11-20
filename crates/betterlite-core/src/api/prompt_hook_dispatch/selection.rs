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
pub fn select_prompt_manager<BackendError: Error + Send + Sync + 'static>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    request: &PromptSelectionRequest<'_>,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>> {
    // Step 1: a known compatible callback name the model starts with wins, and its init decides.
    for name in dispatch.factory.known_callbacks() {
        if !request.model.starts_with(*name) {
            continue;
        }
        let Some(manager) = dispatch.factory.init(name) else {
            continue;
        };
        let prefix = request
            .model
            .split_once('/')
            .map_or(request.model, |(head, _)| head);
        write_stamp(dispatch, prefix);
        return Some(manager);
    }
    // Steps 2 and 3: the prompt id and the dynamic params decide between auto-detection and the fallback.
    if let Some(manager) = detected_or_fallback(dispatch.managers.as_slice(), request) {
        return Some(stamped(dispatch, manager));
    }
    // Step 4: B4126's selector, which needs a truthy cache_control_injection_points.
    if let Some(manager) =
        cache_control_custom_logger(&prompt_object(request.non_default_params), |name| {
            dispatch.factory.init(name)
        })
    {
        return Some(stamped(dispatch, manager));
    }
    // Step 5: a configured registry ends the search, and its hook joins the global callback list once.
    dispatch.vector_stores?;
    let manager = dispatch.factory.init(VECTOR_STORE_INTEGRATION)?;
    if !dispatch
        .callbacks
        .iter()
        .any(|existing| Arc::ptr_eq(existing, &manager))
    {
        dispatch.callbacks.push(Arc::clone(&manager));
    }
    Some(stamped(dispatch, manager))
}

// Steps 2 and 3: a prompt id and dynamic params pick the first manager that runs, and every other case falls back.
fn detected_or_fallback<BackendError: Error + Send + Sync + 'static>(
    managers: &[Arc<dyn PromptManager<BackendError> + Send + Sync>],
    request: &PromptSelectionRequest<'_>,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>> {
    if let Some(prompt_id) = request.prompt_id.filter(|id| !id.is_empty())
        && let Some(dynamic) = request.dynamic_callback_params
        && let Some(manager) = first_running_manager(managers, Some(prompt_id), request, dynamic)
    {
        return Some(manager);
    }
    if request.prompt_id.is_some() {
        return managers.first().map(Arc::clone);
    }
    let defaults = Json::Object(JsonObject::new());
    let dynamic = request.dynamic_callback_params.unwrap_or(&defaults);
    first_running_manager(managers, None, request, dynamic)
}

fn first_running_manager<BackendError: Error + Send + Sync + 'static>(
    managers: &[Arc<dyn PromptManager<BackendError> + Send + Sync>],
    prompt_id: Option<&str>,
    request: &PromptSelectionRequest<'_>,
    dynamic_callback_params: &StandardCallbackDynamicParams,
) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>> {
    managers
        .iter()
        .find(|manager| {
            manager
                .should_run(prompt_id, request.prompt_spec, dynamic_callback_params)
                .unwrap_or(false)
        })
        .map(Arc::clone)
}

fn stamped<BackendError: Error + Send + Sync + 'static>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    manager: Arc<dyn PromptManager<BackendError> + Send + Sync>,
) -> Arc<dyn PromptManager<BackendError> + Send + Sync> {
    write_stamp(dispatch, manager.integration_name());
    manager
}

fn write_stamp<BackendError: Error + Send + Sync + 'static>(
    dispatch: &mut PromptHookDispatch<'_, BackendError>,
    integration: &str,
) {
    dispatch.state.model_call_details.insert(
        INTEGRATION_KEY.to_owned(),
        Json::Text(integration.to_owned()),
    );
}

mod tests;

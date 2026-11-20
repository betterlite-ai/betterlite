use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::litellm_agent_resolver::is_litellm_agent_model;

use crate::api::vector_store_registry::store::VectorStoreRegistry;
use crate::api::vector_tool_selection::selection::get_vector_store_to_run;

use super::json_bridge::prompt_json;

const AGENT_MODEL_KEY: &str = "model";
const DYNAMIC_PARAM_NAMES: [&str; 3] = [
    "cache_control_injection_points",
    "knowledge_bases",
    "vector_store_ids",
];

// LiteLLM's should_run_prompt_management_hooks; the 30-character name rule shortens it.
#[must_use]
pub fn should_run_prompt_hooks(
    non_default_params: &JsonObject,
    prompt_id: Option<&str>,
    tools: Option<&[JsonObject]>,
    vector_stores: Option<&VectorStoreRegistry>,
) -> bool {
    if prompt_id.is_some_and(|id| !id.is_empty()) {
        return true;
    }
    if runs_for_agent_model(non_default_params) {
        return true;
    }
    let has_dynamic_name = DYNAMIC_PARAM_NAMES.iter().any(|name| {
        non_default_params
            .get(*name)
            .is_some_and(|value| prompt_json(value).is_truthy())
    });
    if has_dynamic_name {
        return true;
    }
    let Some(registry) = vector_stores else {
        return false;
    };
    // A non-string id makes B1062 refuse the list, and the gate's bool answer counts that as no store to run.
    get_vector_store_to_run(registry, non_default_params, tools)
        .ok()
        .flatten()
        .is_some()
}

// LiteLLM reads non_default_params["model"], not the model argument, and only a string can carry the prefix.
fn runs_for_agent_model(non_default_params: &JsonObject) -> bool {
    match non_default_params.get(AGENT_MODEL_KEY) {
        Some(Json::Text(model)) => is_litellm_agent_model(model),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => false,
    }
}

mod tests;

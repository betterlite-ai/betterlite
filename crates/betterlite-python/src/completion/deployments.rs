use super::session::SessionCore;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::completion_request::args::CompletionSettings;
use betterlite_sdk::completion_request::ports::ProviderLookup;
use secrecy::SecretString;
use std::collections::BTreeSet;
// The SDK decides which deployment a call lands on by model id, so a session hands it one per reachable model.
pub(crate) fn all(
    core: &SessionCore,
    arguments: &RequestMap,
    settings: &CompletionSettings,
    globals: &GlobalCredentials,
) -> Vec<ChatDeployment> {
    let mut collected = Collected::default();
    if let Some(deployment) = deployment_of(core, arguments, settings, globals) {
        collected.push(deployment);
    }
    for entry in fallback_entries(arguments, settings) {
        let attempt = fallback_arguments(arguments, &entry);
        if let Some(deployment) = deployment_of(core, &attempt, settings, globals) {
            collected.push(deployment);
        }
    }
    collected.found
}
#[derive(Default)]
struct Collected {
    found: Vec<ChatDeployment>,
    seen: BTreeSet<String>,
}
impl Collected {
    // the dispatch looks a deployment up by model id, so a repeat only keeps the first entry's base and key
    fn push(&mut self, deployment: ChatDeployment) {
        if self.seen.insert(deployment.model_id.clone()) {
            self.found.push(deployment);
        }
    }
}
// get_llm_provider's inputs, resolved once here the way main.py resolves them during the call
fn deployment_of(
    core: &SessionCore,
    arguments: &RequestMap,
    settings: &CompletionSettings,
    globals: &GlobalCredentials,
) -> Option<ChatDeployment> {
    let (model, provider) = request_identity(arguments, settings)?;
    let api_base = request_api_base(arguments);
    let api_key = text_field(arguments, "api_key").map(SecretString::from);
    let lookup = ProviderLookup {
        model: &model,
        custom_llm_provider: provider.as_deref(),
        api_base: api_base.as_deref(),
        api_key: api_key.as_ref(),
    };
    let resolved = core.resolve(&lookup, globals).ok()?;
    let prefix = format!("{}/", resolved.provider.slug());
    let litellm_model = if resolved.model.starts_with(&prefix) {
        resolved.model.clone()
    } else {
        format!("{prefix}{}", resolved.model)
    };
    let declared = ModelDeployment {
        model_name: resolved.model.clone(),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(litellm_model),
            api_base: resolved.api_base.clone(),
            api_key: resolved.api_key.clone(),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        // the dispatch looks a deployment up by its model id, which LiteLLM's model_info.id pins
        model_info: ModelInfo {
            id: Some(resolved.model),
        },
    };
    ChatDeployment::resolve_with(&declared, &core.defaults, None, &core.context(globals)).ok()
}
fn text_field(arguments: &RequestMap, name: &str) -> Option<String> {
    match arguments.get(name)? {
        Json::Text(text) if !text.is_empty() => Some(text.clone()),
        Json::Text(_)
        | Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}
// steps::provider::resolve's order: the alias map, then the azure flag, then deployment_id last
fn request_identity(
    arguments: &RequestMap,
    settings: &CompletionSettings,
) -> Option<(String, Option<String>)> {
    let mut model = text_field(arguments, "model")?;
    if let Some(alias) = settings.model_alias_map.get(&model) {
        model = alias.clone();
    }
    let mut provider = if matches!(arguments.get("azure"), Some(Json::Bool(true))) {
        Some(String::from("azure"))
    } else {
        text_field(arguments, "custom_llm_provider")
    };
    if let Some(deployment) = text_field(arguments, "deployment_id") {
        model = deployment;
        provider = Some(String::from("azure"));
    }
    Some((model, provider))
}
// steps::params::router_params takes base_url first and the api_base kwarg otherwise
fn request_api_base(arguments: &RequestMap) -> Option<String> {
    if let Some(Json::Text(base)) = arguments.get("base_url") {
        return Some(base.clone());
    }
    if let Some(Json::Text(base)) = arguments.get("api_base") {
        return Some(base.clone());
    }
    None
}
fn fallback_entries(arguments: &RequestMap, settings: &CompletionSettings) -> Vec<Json> {
    match arguments.get("fallbacks") {
        None | Some(Json::Null) => settings.model_fallbacks.clone().unwrap_or_default(),
        Some(Json::List(items)) => items.clone(),
        Some(Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => Vec::new(),
    }
}
// completion_call::fallbacks::run_fallbacks removes model and fallbacks, then overlays the entry
fn fallback_arguments(arguments: &RequestMap, entry: &Json) -> RequestMap {
    let mut attempt = arguments.clone();
    attempt.shift_remove("model");
    attempt.shift_remove("fallbacks");
    match entry {
        Json::Text(model) => {
            attempt.insert(String::from("model"), Json::Text(model.clone()));
        }
        Json::Object(fields) => {
            for (name, value) in fields {
                attempt.insert(name.clone(), value.clone());
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
    }
    attempt
}

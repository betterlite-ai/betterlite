use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_context::bedrock_model::regions::AmazonBedrockGlobalConfig;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rand::Rng;
use rand::RngExt as _;
use serde_json::Number;

use super::max_tokens::resolve_max_tokens;
use super::mode::REASONING_EFFORT_MODES;
use super::mode::resolve_health_check_mode;
use super::mode::should_inject_max_tokens;
use super::model_info::HealthCheckModelInfo;
use super::model_settings::HealthSettingsError;
use super::model_settings::ModelHealthSettings;

const HEALTH_CHECK_MESSAGES: [&str; 2] = ["Hey how's it going?", "What's 1 + 1?"];
const DEFAULT_HEALTH_CHECK_VOICE: &str = "alloy";

fn health_check_message<R>(random: &mut R) -> Json
where
    R: Rng,
{
    let roll = random.random_range(0..HEALTH_CHECK_MESSAGES.len());
    let content = HEALTH_CHECK_MESSAGES.get(roll).copied().unwrap_or_default();
    let message = [
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::Text(content.to_owned())),
    ]
    .into_iter()
    .collect();
    Json::List(vec![Json::Object(message)])
}

// The probe starts from the deployment's own params; the catalog's copy is never changed
fn merged_params(litellm_params: &JsonObject, health: &ModelHealthSettings) -> JsonObject {
    let mut params = litellm_params.clone();
    if let Some(overrides) = &health.params {
        for (key, value) in overrides {
            params.insert(key.clone(), value.clone());
        }
    }
    params
}

fn apply_model_overrides(
    params: &mut JsonObject,
    health: &ModelHealthSettings,
    mode: Option<&str>,
) {
    if let Some(model) = &health.model {
        params.insert("model".to_owned(), Json::Text(model.clone()));
    }
    if mode == Some("audio_speech") {
        let voice = health
            .voice
            .clone()
            .unwrap_or_else(|| DEFAULT_HEALTH_CHECK_VOICE.to_owned());
        params.insert("voice".to_owned(), Json::Text(voice));
    }
}

// Health checks bypass get_llm_provider, so a bedrock/region/model id would reach AWS with the region in it
fn strip_bedrock_region_routing(params: &mut JsonObject) {
    let Some(Json::Text(model)) = params.get("model") else {
        return;
    };
    if !model.starts_with("bedrock/") {
        return;
    }
    let regions = AmazonBedrockGlobalConfig.get_all_regions();
    let unstripped = model.strip_prefix("bedrock/").unwrap_or(model);
    let kept: Vec<&str> = unstripped
        .split('/')
        .filter(|part| !regions.contains(part))
        .collect();
    let stripped = kept.join("/");
    params.insert("model".to_owned(), Json::Text(stripped));
    if matches!(params.get("custom_llm_provider"), None | Some(Json::Null)) {
        params.insert(
            "custom_llm_provider".to_owned(),
            Json::Text("bedrock".to_owned()),
        );
    }
}

pub fn update_params_for_health_check<M, R>(
    model_info: &JsonObject,
    litellm_params: &JsonObject,
    policy: &ProbePolicy,
    models: &M,
    random: &mut R,
) -> Result<JsonObject, HealthSettingsError>
where
    M: HealthCheckModelInfo,
    R: Rng,
{
    let health = ModelHealthSettings::parse(model_info)?;
    let mode = resolve_health_check_mode(model_info, litellm_params, models);
    let mut params = merged_params(litellm_params, &health);
    params.insert("messages".to_owned(), health_check_message(random));
    if should_inject_max_tokens(&health, mode.as_deref())
        && let Some(tokens) = resolve_max_tokens(&health, &params, policy, models)
    {
        params.insert("max_tokens".to_owned(), Json::Number(Number::from(tokens)));
    }
    if REASONING_EFFORT_MODES.contains(&mode.as_deref())
        && let Some(effort) = &health.reasoning_effort
    {
        params.insert("reasoning_effort".to_owned(), effort.clone());
    }
    apply_model_overrides(&mut params, &health, mode.as_deref());
    strip_bedrock_region_routing(&mut params);
    Ok(params)
}

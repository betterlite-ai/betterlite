use betterlite_models::decisions::Json;
use betterlite_models::lookup::cost_map_keys::backend_cost_map_keys;
use betterlite_models::model_registration::error::RegistrationWarning;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelCostSource, ModelInfoLookup, ModelRegistry,
    RegisterOptions, RegistrationEnvironment,
};
use betterlite_models::request_pricing::fields::{
    shared_backend_model_info, strip_custom_pricing_fields,
};

use super::PricingInputs;
use super::deployment::{Deployment, DeploymentTarget};
use super::error::DeploymentPricingError;
use super::payload::deployment_cost_payload;

fn log_warnings(warnings: Vec<RegistrationWarning>) {
    for RegistrationWarning::MissingCachePricing { model } in warnings {
        tracing::warn!(
            "register_model: model={model} has custom pricing but not in built-in cost map and no prefix/region variant matched; cache_creation_input_token_cost and cache_read_input_token_cost will default to 0 for this model (input/output cost tracking is unaffected). To track cache cost, add them to model_info"
        );
    }
}

fn mode_text(mode: &Json) -> String {
    match mode {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(mode).unwrap_or_default()
        }
    }
}

// A responses backend mode stays when a deployment says chat, and any existing mode stays when the deployment names none.
fn keep_shared_mode(shared_info: &mut JsonObject, existing_mode: Option<&Json>, backend_key: &str) {
    let Some(existing) = existing_mode.filter(|mode| !matches!(mode, Json::Null)) else {
        return;
    };
    let deployment_mode = shared_info
        .get("mode")
        .filter(|mode| !matches!(mode, Json::Null));
    let responses_downgraded_to_chat = matches!(existing, Json::Text(text) if text == "responses")
        && matches!(deployment_mode, Some(Json::Text(text)) if text == "chat");
    if !responses_downgraded_to_chat && deployment_mode.is_some() {
        return;
    }
    if let Some(deployed) = deployment_mode {
        tracing::warn!(
            "Router: preserving existing mode={} for shared backend key {backend_key} instead of the deployment-specified mode={} (prevents alias registration from downgrading the shared backend mode).",
            mode_text(existing),
            mode_text(deployed),
        );
    }
    shared_info.insert("mode".to_owned(), existing.clone());
}

async fn register_entries<Lookup, Fetcher>(
    registry: &mut ModelRegistry,
    environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
    entries: ModelCostEntries,
    options: &RegisterOptions<'_>,
) -> Result<(), DeploymentPricingError<Fetcher::Error>>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
{
    let registered = registry
        .register_model(ModelCostSource::Entries(entries), options, environment)
        .await?;
    log_warnings(registered.warnings);
    Ok(())
}

pub async fn register_in_cost_map<Lookup, Fetcher>(
    registry: &mut ModelRegistry,
    environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
    inputs: &PricingInputs,
    target: &DeploymentTarget<'_>,
    model_info: &JsonObject,
) -> Result<(), DeploymentPricingError<Fetcher::Error>>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
{
    let stripped;
    let priced = if (inputs.is_strategy_router)(target.model) {
        stripped = strip_custom_pricing_fields(model_info);
        &stripped
    } else {
        model_info
    };
    if let Some(model_id) = target.model_id {
        let by_id_options = RegisterOptions {
            persist_across_reloads: false,
            warning_display_name: Some(target.model),
        };
        let mut entries = ModelCostEntries::new();
        entries.insert(model_id.to_owned(), priced.clone());
        register_entries(registry, environment, entries, &by_id_options).await?;
    }
    let backend_keys = backend_cost_map_keys(target.model, target.custom_llm_provider);
    let mut shared_info = shared_backend_model_info(priced);
    if let Some(backend_key) = backend_keys.first() {
        let existing_mode = match registry.model_cost.get(backend_key) {
            Some(Json::Object(entry)) => entry.get("mode"),
            _ => None,
        };
        keep_shared_mode(&mut shared_info, existing_mode, backend_key);
    }
    let entries: ModelCostEntries = backend_keys
        .into_iter()
        .map(|key| (key, shared_info.clone()))
        .collect();
    let shared_options = RegisterOptions {
        persist_across_reloads: false,
        warning_display_name: None,
    };
    register_entries(registry, environment, entries, &shared_options).await
}

pub async fn register_deployment_pricing<Lookup, Fetcher>(
    registry: &mut ModelRegistry,
    litellm_params: &JsonObject,
    model_info: &JsonObject,
    environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
    inputs: &PricingInputs,
) -> Result<(), DeploymentPricingError<Fetcher::Error>>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
{
    let deployment = Deployment::read(litellm_params, model_info)?;
    let payload = deployment_cost_payload(
        litellm_params,
        model_info,
        registry,
        environment.lookup,
        inputs,
    )
    .await?;
    register_in_cost_map(registry, environment, inputs, &deployment.target, &payload).await
}

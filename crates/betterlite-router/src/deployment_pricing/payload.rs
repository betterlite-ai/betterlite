use betterlite_models::lookup::cost_map_keys::cost_map_backend_model;
use betterlite_models::model_registration::{JsonObject, ModelInfoLookup, ModelRegistry};
use betterlite_models::request_pricing::custom_fields::CUSTOM_PRICING_FIELDS;

use super::PricingInputs;
use super::deployment::{Deployment, DeploymentReadError};
use super::inherit::{BackendModel, BackendSource, is_set};

pub async fn deployment_cost_payload<Lookup>(
    litellm_params: &JsonObject,
    model_info: &JsonObject,
    registry: &mut ModelRegistry,
    lookup: &Lookup,
    inputs: &PricingInputs,
) -> Result<JsonObject, DeploymentReadError>
where
    Lookup: ModelInfoLookup,
{
    let deployment = Deployment::read(litellm_params, model_info)?;
    let mut payload = model_info.clone();
    for field in CUSTOM_PRICING_FIELDS {
        if is_set(litellm_params, field)
            && let Some(value) = litellm_params.get(*field)
        {
            payload.insert((*field).to_owned(), value.clone());
        }
    }
    let backend = BackendModel {
        model: deployment.target.model,
        custom_llm_provider: deployment.target.custom_llm_provider,
    };
    BackendSource::new(registry, lookup, backend)
        .inherit_off_peak_rates(&mut payload)
        .await;
    if is_set(&payload, "input_cost_per_token") {
        BackendSource::new(registry, lookup, backend)
            .inherit_cache_pricing(&mut payload)
            .await;
    }
    let tier_backend = BackendModel {
        model: cost_map_backend_model(litellm_params, model_info).unwrap_or(backend.model),
        custom_llm_provider: backend.custom_llm_provider,
    };
    let mut tier_source = BackendSource::new(registry, lookup, tier_backend);
    tier_source
        .inherit_service_tier_pricing(&mut payload, inputs)
        .await;
    BackendSource::new(registry, lookup, backend)
        .inherit_tiered_output_rate(&mut payload)
        .await;
    Ok(payload)
}

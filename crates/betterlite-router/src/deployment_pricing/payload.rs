use betterlite_models::lookup::cost_map_keys::cost_map_backend_model;
use betterlite_models::model_registration::{JsonObject, ModelInfoLookup, ModelRegistry};
use betterlite_models::request_pricing::custom_fields::CUSTOM_PRICING_FIELDS;

use super::PricingInputs;
use super::deployment::{Deployment, DeploymentReadError};
use super::inherit::{
    BackendModel, inherit_cache_pricing, inherit_off_peak_rates, inherit_service_tier_pricing,
    inherit_tiered_output_rate, is_set,
};

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
    inherit_off_peak_rates(&mut payload, registry, lookup, &backend).await;
    if is_set(&payload, "input_cost_per_token") {
        inherit_cache_pricing(&mut payload, registry, lookup, &backend).await;
    }
    let tier_backend = BackendModel {
        model: cost_map_backend_model(litellm_params, model_info).unwrap_or(backend.model),
        custom_llm_provider: backend.custom_llm_provider,
    };
    inherit_service_tier_pricing(&mut payload, registry, lookup, &tier_backend, inputs).await;
    inherit_tiered_output_rate(&mut payload, registry, lookup, &backend).await;
    Ok(payload)
}

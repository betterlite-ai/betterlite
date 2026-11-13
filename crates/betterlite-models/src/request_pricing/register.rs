use super::entry::{build_custom_pricing_entry, router_deployment_id};
use super::error::RequestPricingError;
use super::fields::strip_custom_pricing_fields;
use crate::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelCostSource, ModelInfoLookup, ModelRegistry,
    RegisterOptions, Registered, RegistrationEnvironment,
};

#[derive(Clone, Copy, Debug)]
pub struct CustomPricingRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub kwargs: &'request JsonObject,
    pub model_info: Option<&'request JsonObject>,
}

pub async fn register_custom_pricing<Lookup, Fetcher>(
    registry: &mut ModelRegistry,
    environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
    request: &CustomPricingRequest<'_>,
) -> Result<Registered, RequestPricingError<Fetcher::Error>>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
{
    let entry = build_custom_pricing_entry(
        request.custom_llm_provider,
        request.kwargs,
        request.model_info,
    );
    let shared_key = format!("{}/{}", request.custom_llm_provider, request.model);
    let mut entries = ModelCostEntries::new();
    let mut options = RegisterOptions {
        persist_across_reloads: false,
        warning_display_name: None,
    };
    if let Some(deployment_id) = router_deployment_id(request.kwargs)? {
        entries.insert(deployment_id, entry.clone());
        entries.insert(shared_key.clone(), strip_custom_pricing_fields(&entry));
        options.warning_display_name = Some(&shared_key);
    } else {
        entries.insert(shared_key.clone(), entry);
    }
    let registered = registry
        .register_model(ModelCostSource::Entries(entries), &options, environment)
        .await?;
    Ok(registered)
}

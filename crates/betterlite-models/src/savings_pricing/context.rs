use std::future::Future;

use crate::price_lookup::{PriceLookup, PricedModel};

// _ModelIdentity: the canonical name and provider a comparison is made on, never the spellings that arrived.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelIdentity {
    pub model: String,
    pub provider: String,
}

// The proxy side of savings pricing, injected so this crate never reaches Router or proxy state.
pub trait SavingsPricingContext: Send + Sync {
    // _resolve_model's get_llm_provider call: None when the name resolves to no provider.
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity>;

    // _effective_model_info: what a deployment is charged, or None when no deployment answers for the id.
    fn deployment_price(
        &self,
        deployment_id: Option<&str>,
        model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send;

    // The B1678 lookup public prices come from, shared with the rest of the cost resource.
    fn public_lookup(&self) -> &dyn PriceLookup;
}

// _resolve_model: an absent or empty name resolves to no identity.
#[must_use]
pub fn resolve_model<C>(
    context: &C,
    model: Option<&str>,
    custom_llm_provider: Option<&str>,
) -> Option<ModelIdentity>
where
    C: SavingsPricingContext,
{
    let name = model.filter(|value| !value.is_empty())?;
    let provider = custom_llm_provider.filter(|value| !value.is_empty());
    let identity = context.resolve_model(name, provider);
    if identity.is_none() {
        tracing::debug!(
            model = name,
            custom_llm_provider = ?provider,
            "savings: cannot resolve a provider"
        );
    }
    identity
}

// _model_info: the public rates for an identity, or None when the lookup has none.
#[must_use]
pub fn public_price<C>(context: &C, identity: &ModelIdentity) -> Option<PricedModel>
where
    C: SavingsPricingContext,
{
    let pricing = context
        .public_lookup()
        .effective_priced_model(&identity.model, Some(&identity.provider));
    if pricing.is_none() {
        tracing::debug!(
            model = identity.model,
            provider = identity.provider,
            "savings: no public pricing"
        );
    }
    pricing
}

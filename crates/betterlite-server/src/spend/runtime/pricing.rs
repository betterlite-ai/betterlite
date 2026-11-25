use std::future::Future;
use std::pin::Pin;

use betterlite_models::price_lookup::{PriceLookup, PricedModel};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use betterlite_provider::pricing::grounding::ProviderFormulas;
use jiff::tz::TimeZoneDatabase;

use crate::cost_service::CostService;
use crate::savings;

pub type PriceFuture<'call> = Pin<Box<dyn Future<Output = Option<PricedModel>> + Send + 'call>>;

// The pricing B1743's daily builders read for one runtime: identities, public rates, deployment prices and the request's zone.
pub trait SpendPricingSource: Send + Sync {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity>;

    fn deployment_price(&self, deployment_id: Option<&str>, model: &str) -> PriceFuture<'_>;

    fn public_lookup(&self) -> &dyn PriceLookup;

    fn zone(&self) -> &TimeZoneDatabase;
}

// The one cost resource the server builds from the bundled map; a deployment's own price needs the router this resource does not bind.
impl SpendPricingSource for CostService<ProviderFormulas> {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        let context = savings::Context {
            lookup: &self.lookup,
            deployments: None,
        };
        SavingsPricingContext::resolve_model(&context, model, custom_llm_provider)
    }

    fn deployment_price(&self, deployment_id: Option<&str>, model: &str) -> PriceFuture<'_> {
        let owned_deployment = deployment_id.map(ToOwned::to_owned);
        let owned_model = model.to_owned();
        Box::pin(async move {
            let context = savings::Context {
                lookup: &self.lookup,
                deployments: None,
            };
            SavingsPricingContext::deployment_price(
                &context,
                owned_deployment.as_deref(),
                &owned_model,
            )
            .await
        })
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        &self.lookup
    }

    fn zone(&self) -> &TimeZoneDatabase {
        &self.database
    }
}

// The savings context the daily builders take over the runtime's pricing source.
pub(super) struct PricingView<'source>(pub(super) &'source dyn SpendPricingSource);

impl SavingsPricingContext for PricingView<'_> {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        self.0.resolve_model(model, custom_llm_provider)
    }

    fn deployment_price(
        &self,
        deployment_id: Option<&str>,
        model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send {
        self.0.deployment_price(deployment_id, model)
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        self.0.public_lookup()
    }
}

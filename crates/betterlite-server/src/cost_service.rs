use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::cost_adjustments::{AdjustmentEntry, Settings as AdjustmentSettings};
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::cost::call_price::policy::ProviderPricePolicy;
use betterlite_core::cost::completion::args::{CompletionArgs, CostCalculator};
use betterlite_core::cost::completion::error::CompletionCostError;
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupError, LookupSettings};
use betterlite_core::cost::completion::{completion_cost, reading};
use betterlite_core::cost::image_dispatch::error::ImageDispatchError;
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_core::cost::response::response_cost_calculator;
use betterlite_core::cost::special::policy::SpecialPricePolicy;
use betterlite_core::cost::special::request::SpecialServices;
use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::tool_fees::billing::BuiltInToolBilling;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_provider::pricing::ProviderPricing;
use betterlite_provider::pricing::grounding::{
    GroundingFormulas, GroundingPricing, ProviderFormulas,
};
use betterlite_provider::pricing::image::ProviderImagePricing;
use betterlite_provider::pricing::image::error::ImagePolicyError;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

mod tests;

// DEFAULT_REPLICATE_GPU_PRICE_PER_SECOND: every Replicate call is priced as an A100 80GB second.
const REPLICATE_PRICE_PER_SECOND: Decimal = Decimal::from_parts(14, 0, 0, false, 4);

#[derive(Debug, thiserror::Error)]
pub enum CostServiceError {
    #[error("the price lookup could not be built: {0}")]
    Lookup(#[from] LookupError),
    #[error("the image size pattern could not be built: {0}")]
    Pattern(#[from] ImageDispatchError),
}

// The price formulas one deployment installs: a later provider replaces its own entry and every other operation keeps its typed unavailable answer.
pub struct Policies<Formulas> {
    pub price: Box<dyn ProviderPricePolicy + Send + Sync>,
    pub special: Box<dyn SpecialPricePolicy + Send + Sync>,
    pub formulas: Formulas,
}

impl Policies<ProviderFormulas> {
    #[must_use]
    pub fn native() -> Self {
        Self {
            price: Box::new(ProviderPricing),
            special: Box::new(ProviderPricing),
            formulas: ProviderFormulas,
        }
    }
}

// The bedrock helpers the model-info resolver takes from the crates above it.
#[must_use]
pub fn name_normalizers() -> NameNormalizers {
    NameNormalizers {
        bedrock_base_model: get_bedrock_base_model,
        bedrock_routing_prefix: strip_bedrock_routing_prefix,
        mantle_region_split: split_region_prefix,
    }
}

// The one cost resource the request owners share: the catalog bound to its effective lookup, the installed policies and the configured adjustments.
pub struct CostService<Formulas> {
    pub lookup: CatalogLookup,
    policies: Policies<Formulas>,
    pattern: SizePattern,
    models: ProviderModelSets,
    database: TimeZoneDatabase,
    discount: BTreeMap<String, AdjustmentEntry>,
    margin: BTreeMap<String, AdjustmentEntry>,
}

// One charge to price: the call's inputs and the token counter of its model.
pub struct Call<'call, 'args> {
    pub args: &'call CompletionArgs<'args>,
    pub counter: &'call dyn BillingTokenCounter,
}

// What one call borrows besides the service: the image and grounding formulas that read the call's moment and model.
struct Bound<'bound, Formulas> {
    image: ProviderImagePricing<'bound>,
    tools: GroundingPricing<'bound, Formulas>,
    resolution: ResolutionContext<'bound>,
}

impl<Formulas> CostService<Formulas>
where
    Formulas: GroundingFormulas + Copy + 'static,
{
    pub fn new(
        model_cost: &[u8],
        settings: LookupSettings,
        adjustments: &AdjustmentSettings,
        policies: Policies<Formulas>,
    ) -> Result<Self, CostServiceError> {
        let lookup = CatalogLookup::new(model_cost, settings)?;
        let mut models = ProviderModelSets::default();
        models.add_known_models(lookup.resolution().catalog);
        Ok(Self {
            lookup,
            policies,
            pattern: SizePattern::new()?,
            models,
            database: TimeZoneDatabase::from_env(),
            discount: adjustments.cost_discount_config.clone(),
            margin: adjustments.cost_margin_config.clone(),
        })
    }

    fn calculator<'call>(
        &'call self,
        bound: &'call Bound<'call, Formulas>,
        counter: &'call dyn BillingTokenCounter,
    ) -> CostCalculator<'call, CatalogLookup, ProviderImagePricing<'call>, CatalogLookup> {
        CostCalculator {
            special: SpecialServices {
                lookup: &self.lookup,
                resolver: &self.lookup,
                image_policy: &bound.image,
                special_policy: &*self.policies.special,
                size_pattern: &self.pattern,
                database: &self.database,
                replicate_models: &self.models,
                replicate_price: REPLICATE_PRICE_PER_SECOND,
                discount: &self.discount,
                margin: &self.margin,
            },
            catalog: &self.lookup,
            inference: &self.lookup,
            counter,
            price_policy: &*self.policies.price,
            resolution: &bound.resolution,
            tools: BuiltInToolBilling {
                source: &self.lookup,
                policy: &bound.tools,
                prices: &ToolPrices::PINNED,
            },
        }
    }

    fn bound<'call, Clock>(
        &'call self,
        model_key: Option<&'call str>,
        clock: &mut Clock,
    ) -> Result<Bound<'call, Formulas>, CompletionCostError<ImagePolicyError>>
    where
        Clock: FnMut() -> SystemTime,
    {
        Ok(Bound {
            image: ProviderImagePricing {
                lookup: &self.lookup,
                database: &self.database,
                moment: reading(clock)?,
            },
            tools: GroundingPricing {
                formulas: self.policies.formulas,
                model_key,
            },
            resolution: self.lookup.resolution(),
        })
    }

    // completion_cost with every collaborator this resource owns.
    pub async fn completion_cost<Target, Clock>(
        &self,
        call: &Call<'_, '_>,
        clock: &mut Clock,
        target: Option<&mut Target>,
    ) -> Result<Decimal, CompletionCostError<ImagePolicyError>>
    where
        Target: CostBreakdownTarget,
        Clock: FnMut() -> SystemTime,
    {
        let bound = self.bound(call.args.billing.model, clock)?;
        let calculator = self.calculator(&bound, call.counter);
        completion_cost(&calculator, call.args, clock, target).await
    }

    // response_cost_calculator with every collaborator this resource owns.
    pub async fn response_cost<Target, Clock>(
        &self,
        call: &Call<'_, '_>,
        cache_hit: Option<bool>,
        clock: &mut Clock,
        target: Option<&mut Target>,
    ) -> Result<Decimal, CompletionCostError<ImagePolicyError>>
    where
        Target: CostBreakdownTarget,
        Clock: FnMut() -> SystemTime,
    {
        let bound = self.bound(call.args.billing.model, clock)?;
        let calculator = self.calculator(&bound, call.counter);
        response_cost_calculator(&calculator, call.args, cache_hit, clock, target).await
    }
}

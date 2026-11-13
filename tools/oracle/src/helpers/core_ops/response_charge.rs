use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::cost::completion::args::{CompletionArgs, CostCalculator};
use betterlite_core::cost::completion::completion_cost;
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_core::cost::prepare::{
    BillingTokenCounter, CompletionBillingInput, ResponseView, TypedResponse,
};
use betterlite_core::cost::response::response_cost_calculator;
use betterlite_core::cost::special::request::SpecialServices;
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::tool_fees::billing::BuiltInToolBilling;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_provider::pricing::ProviderPricing;
use betterlite_provider::pricing::grounding::{GroundingPricing, ProviderFormulas};
use betterlite_provider::pricing::image::ProviderImagePricing;
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::json;

use self::input::{Function, Input, adjustments, charge_inputs, hidden_params, usage_block};

mod input;

struct Counter;

#[async_trait]
impl BillingTokenCounter for Counter {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _model: &str, _text: &str) -> Result<usize, CountError> {
        Ok(0)
    }
}

struct Silent;

impl CostBreakdownTarget for Silent {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, _input: &CostBreakdownInput) -> Result<(), Infallible> {
        Ok(())
    }
}

fn failure(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

async fn charge(input: &Input) -> Result<Decimal, Box<crate::helpers::HelperError>> {
    let catalog_bytes = serde_json::to_vec(&input.catalog)
        .map_err(|error| failure("CatalogError", error.to_string()))?;
    let lookup = CatalogLookup::new(
        &catalog_bytes,
        LookupSettings {
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            normalizers: NameNormalizers {
                bedrock_base_model: get_bedrock_base_model,
                bedrock_routing_prefix: strip_bedrock_routing_prefix,
                mantle_region_split: split_region_prefix,
            },
        },
    )
    .map_err(|error| failure("CatalogError", error.to_string()))?;
    let mut clock = || UNIX_EPOCH;
    let (discount, margin) = adjustments(input);
    let database = TimeZoneDatabase::from_env();
    let pattern = SizePattern::new().map_err(|error| failure("PatternError", error.to_string()))?;
    let models = ProviderModelSets::default();
    let image = ProviderImagePricing {
        lookup: &lookup,
        database: &database,
        moment: DateTime::<Utc>::UNIX_EPOCH,
    };
    let tools = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: Some(input.model.as_str()),
    };
    let resolution = lookup.resolution();
    let calculator = CostCalculator {
        special: SpecialServices {
            lookup: &lookup,
            resolver: &lookup,
            image_policy: &image,
            special_policy: &ProviderPricing,
            size_pattern: &pattern,
            database: &database,
            replicate_models: &models,
            replicate_price: Decimal::new(14, 4),
            discount: &discount,
            margin: &margin,
        },
        catalog: &lookup,
        inference: &lookup,
        counter: &Counter,
        price_policy: &ProviderPricing,
        resolution: &resolution,
        tools: BuiltInToolBilling {
            source: &lookup,
            policy: &tools,
            prices: &ToolPrices::PINNED,
        },
    };
    let usage = usage_block(&input.usage);
    let hidden = input.header_cost.as_ref().map(hidden_params);
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: Some(input.model.as_str()),
        usage: BillingUsage::Chat(&usage),
        service_tier: None,
        hidden_params: hidden.as_ref(),
    });
    let args = CompletionArgs {
        billing: CompletionBillingInput {
            model: Some(input.model.as_str()),
            prompt: "",
            messages: &[],
            completion: "",
            call_type: None,
            custom_llm_provider: input.custom_llm_provider.as_deref(),
            region_name: input.region_name.as_deref(),
            router_model_id: None,
            custom_pricing: None,
            base_model: None,
            optional_params: None,
            service_tier: None,
            response: Some(&view),
        },
        charge: charge_inputs(input),
    };
    let found = match input.function {
        Function::CompletionCost => {
            completion_cost(&calculator, &args, &mut clock, None::<&mut Silent>).await
        }
        Function::ResponseCostCalculator => {
            response_cost_calculator(
                &calculator,
                &args,
                input.cache_hit,
                &mut clock,
                None::<&mut Silent>,
            )
            .await
        }
    };
    found.map_err(|error| failure("CompletionCostError", error.to_string()))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        let cost = runtime.block_on(charge(input))?;
        Ok(json!({"cost": cost.to_f64()}))
    }
}

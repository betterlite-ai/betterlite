use std::collections::{BTreeMap, BTreeSet};

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::cost_adjustments::AdjustmentEntry;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::cost::completion::args::{
    CostCalculator, SpecialBorrows, SyncImage, ToolBorrows,
};
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_provider::pricing::ProviderPricing;
use betterlite_provider::pricing::grounding::{GroundingPricing, ProviderFormulas};
use betterlite_provider::pricing::image::ProviderImagePricing;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::adapters::{Counter, SearchPricing, failure};
use super::input::Input;
use super::input::values;
use betterlite_oracle_helper_api::error::HelperError;

pub(super) struct Stage {
    lookup: CatalogLookup,
    discount: BTreeMap<String, AdjustmentEntry>,
    margin: BTreeMap<String, AdjustmentEntry>,
    database: TimeZoneDatabase,
    pattern: SizePattern,
    models: ProviderModelSets,
    search_rates: BTreeMap<String, Decimal>,
}

pub(super) fn build_stage(input: &Input) -> Result<Stage, Box<HelperError>> {
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
    let (discount, margin) = values::adjustments(input);
    Ok(Stage {
        lookup,
        discount,
        margin,
        database: TimeZoneDatabase::from_env(),
        pattern: SizePattern::new().map_err(|error| failure("PatternError", error.to_string()))?,
        models: ProviderModelSets::default(),
        search_rates: values::search_rates(&input.catalog),
    })
}

pub(super) struct Adapters<'stage> {
    image: SyncImage<'stage, ProviderImagePricing<'stage>>,
    tools: GroundingPricing<'stage, ProviderFormulas>,
    search: SearchPricing<'stage>,
    resolution: ResolutionContext<'stage>,
}

pub(super) fn build_adapters<'stage>(
    stage: &'stage Stage,
    input: &'stage Input,
) -> Adapters<'stage> {
    Adapters {
        image: SyncImage {
            build: |lookup, database, moment| ProviderImagePricing {
                lookup,
                database,
                moment,
            },
            lookup: &stage.lookup,
            database: &stage.database,
            moment: DateTime::<Utc>::UNIX_EPOCH,
        },
        tools: GroundingPricing {
            formulas: ProviderFormulas,
            model_key: Some(input.model.as_str()),
        },
        search: SearchPricing {
            rates: &stage.search_rates,
        },
        resolution: stage.lookup.resolution(),
    }
}

pub(super) fn build_calculator<'deps>(
    stage: &'deps Stage,
    adapters: &'deps Adapters<'deps>,
) -> CostCalculator<
    'deps,
    CatalogLookup,
    SyncImage<'deps, ProviderImagePricing<'deps>>,
    CatalogLookup,
> {
    CostCalculator {
        special: SpecialBorrows {
            lookup: &stage.lookup,
            resolver: &stage.lookup,
            image_policy: &adapters.image,
            special_policy: &adapters.search,
            size_pattern: &stage.pattern,
            database: &stage.database,
            replicate_models: &stage.models,
            replicate_price: Decimal::new(14, 4),
            discount: &stage.discount,
            margin: &stage.margin,
        },
        catalog: &stage.lookup,
        inference: &stage.lookup,
        counter: &Counter,
        price_policy: &ProviderPricing,
        resolution: &adapters.resolution,
        tools: ToolBorrows {
            source: &stage.lookup,
            policy: &adapters.tools,
            prices: &ToolPrices::PINNED,
        },
    }
}

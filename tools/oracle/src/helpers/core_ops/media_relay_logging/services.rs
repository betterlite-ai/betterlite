use std::collections::{BTreeMap, BTreeSet};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::cost_adjustments::AdjustmentEntry;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::media_relay_logging::deepgram::{
    DeepgramListenOutcome, DeepgramListenRequest, deepgram_listen_outcome,
};
use betterlite_core::cost::completion::args::{
    CostCalculator, SpecialBorrows, SyncImage, ToolBorrows,
};
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::pricing::ProviderPricing;
use betterlite_provider::pricing::grounding::{GroundingPricing, ProviderFormulas};
use betterlite_provider::pricing::image::ProviderImagePricing;
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

// The case's catalog stands in for litellm.model_cost for the Fal cost, where a raw lookup is the key and an effective one is provider-qualified.
pub(super) struct CaseLookup {
    entries: BTreeMap<String, PriceRecord>,
}

impl PriceLookup for CaseLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        let scoped =
            custom_llm_provider.and_then(|name| self.entries.get(&format!("{name}/{model}")));
        scoped.or_else(|| self.entries.get(model)).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        let record = self.effective_model_info(model, custom_llm_provider)?;
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

pub(super) fn fal_lookup(entries: &IndexMap<String, Json>) -> Result<CaseLookup, Box<HelperError>> {
    let bytes = serde_json::to_vec(entries).map_err(super::failure)?;
    Ok(CaseLookup {
        entries: serde_json::from_slice(&bytes).map_err(super::failure)?,
    })
}

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

// litellm.cost_discount_config and litellm.cost_margin_config as the config reader resolves them.
pub(super) struct Adjustments {
    pub(super) discount: BTreeMap<String, AdjustmentEntry>,
    pub(super) margin: BTreeMap<String, AdjustmentEntry>,
}

// The Deepgram relay handler's pricing over the case's catalog, discount and margin, resolved the way a served request resolves them.
pub(super) async fn listen_outcome(
    entries: &IndexMap<String, Json>,
    adjustments: &Adjustments,
    upstream_url: &str,
    messages: &[Json],
    kwargs: &PassThroughLoggingKwargs,
) -> Result<DeepgramListenOutcome, Box<HelperError>> {
    let bytes = serde_json::to_vec(entries).map_err(super::failure)?;
    let lookup = CatalogLookup::new(
        &bytes,
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
    .map_err(super::failure)?;
    let resolution = lookup.resolution();
    let pattern = SizePattern::new().map_err(super::failure)?;
    let database = TimeZoneDatabase::from_env();
    let models = ProviderModelSets::default();
    let image = SyncImage {
        build: |lookup, database, moment| ProviderImagePricing {
            lookup,
            database,
            moment,
        },
        lookup: &lookup,
        database: &database,
        moment: DateTime::<Utc>::UNIX_EPOCH,
    };
    let tools = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: None,
    };
    let services = CostCalculator {
        special: SpecialBorrows {
            lookup: &lookup,
            resolver: &lookup,
            image_policy: &image,
            special_policy: &ProviderPricing,
            size_pattern: &pattern,
            database: &database,
            replicate_models: &models,
            replicate_price: Decimal::new(14, 4),
            discount: &adjustments.discount,
            margin: &adjustments.margin,
        },
        catalog: &lookup,
        inference: &lookup,
        counter: &Counter,
        price_policy: &ProviderPricing,
        resolution: &resolution,
        tools: ToolBorrows {
            source: &lookup,
            policy: &tools,
            prices: &ToolPrices::PINNED,
        },
    };
    let request = DeepgramListenRequest {
        upstream_url,
        messages,
        kwargs,
        catalog: resolution.catalog,
    };
    let mut clock = || UNIX_EPOCH;
    deepgram_listen_outcome(&request, &services, &mut clock)
        .await
        .map_err(super::failure)
}

use std::collections::{BTreeMap, BTreeSet};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_config::cost_adjustments::AdjustmentEntry;
use betterlite_core::api::media_relay_logging::deepgram::{
    DeepgramListenOutcome, DeepgramListenRequest, deepgram_listen_outcome,
};
use betterlite_core::cost::completion::args::{
    CostCalculator, SpecialBorrows, SyncImage, ToolBorrows,
};
use betterlite_core::cost::context::ProviderInference;
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_core::cost::special::policy::ModelResolver;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::{ProviderIdentity, ResolutionContext, ResolvedModel};
use betterlite_models::tool_fees::error::LookupFailed;
use betterlite_models::tool_fees::model_info::{Rate, ToolModelInfo};
use betterlite_models::tool_fees::ports::ToolPriceSource;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_provider::pricing::ProviderPricing;
use betterlite_provider::pricing::grounding::{GroundingPricing, ProviderFormulas};
use betterlite_provider::pricing::image::ProviderImagePricing;
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use crate::helpers::HelperError;

// The case's catalog stands in for litellm.model_cost, so a raw lookup is the key and an effective one is provider-qualified.
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

// The cases name every model and provider outright, so no name is ever resolved or inferred.
impl ModelResolver for CaseLookup {
    type Failure = &'static str;

    fn resolve_model(&self, _model: &str) -> Result<ResolvedModel, &'static str> {
        Err("the relay cases resolve no model")
    }
}

impl ProviderInference for CaseLookup {
    type Failure = &'static str;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, &'static str> {
        Ok(ProviderIdentity::Custom("deepgram".to_owned()))
    }
}

// A transcription carries no built-in tool, so the source has no tool prices to give.
impl ToolPriceSource for CaseLookup {
    fn model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<ToolModelInfo, LookupFailed> {
        Err(LookupFailed)
    }

    fn container_session_rate(&self, _provider: &str) -> Result<Rate, LookupFailed> {
        Err(LookupFailed)
    }
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

pub(super) struct Registry {
    pub(super) catalog: ModelCatalog,
    pub(super) lookup: CaseLookup,
}

pub(super) fn registry(entries: &IndexMap<String, Json>) -> Result<Registry, Box<HelperError>> {
    let bytes = serde_json::to_vec(entries).map_err(super::failure)?;
    let catalog = ModelCatalog::parse(&bytes, UnknownFields::Reject).map_err(super::failure)?;
    let lookup = CaseLookup {
        entries: serde_json::from_slice(&bytes).map_err(super::failure)?,
    };
    Ok(Registry { catalog, lookup })
}

// litellm.cost_discount_config and litellm.cost_margin_config as the config reader resolves them.
pub(super) struct Adjustments {
    pub(super) discount: BTreeMap<String, AdjustmentEntry>,
    pub(super) margin: BTreeMap<String, AdjustmentEntry>,
}

// The Deepgram relay handler's pricing over the case's catalog, discount and margin.
pub(super) async fn listen_outcome(
    registry: &Registry,
    adjustments: &Adjustments,
    request: &DeepgramListenRequest<'_>,
) -> Result<DeepgramListenOutcome, Box<HelperError>> {
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let pattern = SizePattern::new().map_err(super::failure)?;
    let database = TimeZoneDatabase::from_env();
    let models = ProviderModelSets::default();
    let image = SyncImage {
        build: |lookup, database, moment| ProviderImagePricing {
            lookup,
            database,
            moment,
        },
        lookup: &registry.lookup,
        database: &database,
        moment: DateTime::<Utc>::UNIX_EPOCH,
    };
    let tools = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: None,
    };
    let resolution = ResolutionContext {
        catalog: &registry.catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let services = CostCalculator {
        special: SpecialBorrows {
            lookup: &registry.lookup,
            resolver: &registry.lookup,
            image_policy: &image,
            special_policy: &ProviderPricing,
            size_pattern: &pattern,
            database: &database,
            replicate_models: &models,
            replicate_price: Decimal::new(14, 4),
            discount: &adjustments.discount,
            margin: &adjustments.margin,
        },
        catalog: &registry.lookup,
        inference: &registry.lookup,
        counter: &Counter,
        price_policy: &ProviderPricing,
        resolution: &resolution,
        tools: ToolBorrows {
            source: &registry.lookup,
            policy: &tools,
            prices: &ToolPrices::PINNED,
        },
    };
    let mut clock = || UNIX_EPOCH;
    deepgram_listen_outcome(request, &services, &mut clock)
        .await
        .map_err(super::failure)
}

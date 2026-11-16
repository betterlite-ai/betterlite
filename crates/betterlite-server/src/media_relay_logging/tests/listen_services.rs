#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_config::cost_adjustments::AdjustmentEntry;
use betterlite_core::api::media_relay_logging::deepgram::DeepgramListenRequest;
use betterlite_core::api::media_relay_logging::error::MediaRelayLoggingError;
use betterlite_core::cost::completion::args::{
    CostCalculator, SpecialBorrows, SyncImage, ToolBorrows,
};
use betterlite_core::cost::context::ProviderInference;
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_core::cost::special::policy::ModelResolver;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
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
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::deepgram::deepgram_listen_passthrough_handler;
use super::support::{CaseLookup, Registry};

// The fixtures name every model and provider outright, so no name is ever resolved or inferred.
impl ModelResolver for CaseLookup {
    type Failure = &'static str;

    fn resolve_model(&self, _model: &str) -> Result<ResolvedModel, &'static str> {
        Err("the relay fixtures resolve no model")
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

// litellm.cost_discount_config and litellm.cost_margin_config as the config reader resolves them.
#[derive(Default)]
pub(super) struct Adjustments {
    pub(super) discount: BTreeMap<String, AdjustmentEntry>,
    pub(super) margin: BTreeMap<String, AdjustmentEntry>,
}

// The Deepgram relay handler run against the registry's prices with no discount and no margin.
pub(super) async fn listen(
    registry: &Registry,
    logging_obj: &StreamLoggingObject,
    upstream_url: &str,
    messages: &[Json],
    caller: &PassThroughLoggingKwargs,
) -> Result<HandlerOutcome, MediaRelayLoggingError> {
    listen_adjusted(
        registry,
        &Adjustments::default(),
        logging_obj,
        upstream_url,
        messages,
        caller,
    )
    .await
}

// The same run with the configured adjustments, which every registry price passes through.
pub(super) async fn listen_adjusted(
    registry: &Registry,
    adjustments: &Adjustments,
    logging_obj: &StreamLoggingObject,
    upstream_url: &str,
    messages: &[Json],
    caller: &PassThroughLoggingKwargs,
) -> Result<HandlerOutcome, MediaRelayLoggingError> {
    let request = DeepgramListenRequest {
        upstream_url,
        messages,
        kwargs: caller,
        catalog: &registry.catalog,
    };
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let pattern = SizePattern::new().expect("the size pattern compiles");
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
    deepgram_listen_passthrough_handler(logging_obj, &request, &services, &mut clock).await
}

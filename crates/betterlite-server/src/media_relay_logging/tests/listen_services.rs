#![cfg(test)]

use std::sync::Arc;

use std::collections::{BTreeMap, BTreeSet};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_config::cost_adjustments::AdjustmentEntry;
use betterlite_core::api::media_relay_logging::deepgram::DeepgramListenRequest;
use betterlite_core::api::media_relay_logging::error::MediaRelayLoggingError;
use betterlite_core::cost::completion::args::{
    CostCalculator, SpecialBorrows, SyncImage, ToolBorrows,
};
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_provider::pricing::ProviderPricing;
use betterlite_provider::pricing::grounding::{GroundingPricing, ProviderFormulas};
use betterlite_provider::pricing::image::ProviderImagePricing;
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::deepgram::deepgram_listen_passthrough_handler;
use crate::cost_service::name_normalizers;

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

// The Deepgram relay handler over a catalog-backed lookup, the one a served request prices with.
pub(super) struct Listener {
    lookup: CatalogLookup,
    discount: BTreeMap<String, AdjustmentEntry>,
    margin: BTreeMap<String, AdjustmentEntry>,
    pattern: SizePattern,
    database: TimeZoneDatabase,
    models: ProviderModelSets,
    tools: GroundingPricing<'static, ProviderFormulas>,
}

impl Listener {
    // The entries are the cost map's rows, each a provider-qualified key and its JSON body.
    pub(super) fn new(entries: &[(&str, &str)]) -> Self {
        let mut rows: BTreeMap<String, Json> = BTreeMap::new();
        for (key, body) in entries {
            let row = serde_json::from_str(body).expect("a JSON literal");
            rows.insert((*key).to_owned(), row);
        }
        let bytes = serde_json::to_vec(&rows).expect("the catalog serializes");
        let settings = LookupSettings {
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            normalizers: name_normalizers(),
        };
        Self {
            lookup: CatalogLookup::new(
                &bytes,
                Arc::new(
                    ModelCatalog::parse(&bytes, UnknownFields::Warn).expect("the catalog loads"),
                ),
                settings,
            )
            .expect("the catalog loads"),
            discount: BTreeMap::new(),
            margin: BTreeMap::new(),
            pattern: SizePattern::new().expect("the size pattern compiles"),
            database: TimeZoneDatabase::from_env(),
            models: ProviderModelSets::default(),
            tools: GroundingPricing {
                formulas: ProviderFormulas,
                model_key: None,
            },
        }
    }

    // litellm.cost_discount_config and litellm.cost_margin_config as the config reader resolves them.
    pub(super) fn adjusted(
        mut self,
        discount: &[(&str, AdjustmentEntry)],
        margin: &[(&str, AdjustmentEntry)],
    ) -> Self {
        self.discount = discount
            .iter()
            .map(|(key, entry)| ((*key).to_owned(), *entry))
            .collect();
        self.margin = margin
            .iter()
            .map(|(key, entry)| ((*key).to_owned(), *entry))
            .collect();
        self
    }

    pub(super) async fn listen(
        &self,
        logging_obj: &StreamLoggingObject,
        upstream_url: &str,
        messages: &[Json],
        caller: &PassThroughLoggingKwargs,
    ) -> Result<HandlerOutcome, MediaRelayLoggingError> {
        let resolution = self.lookup.resolution();
        let image = SyncImage {
            build: |lookup, database, moment| ProviderImagePricing {
                lookup,
                database,
                moment,
            },
            lookup: &self.lookup,
            database: &self.database,
            moment: DateTime::<Utc>::UNIX_EPOCH,
        };
        let services = self.calculator(&image, &resolution);
        let request = DeepgramListenRequest {
            upstream_url,
            messages,
            kwargs: caller,
            catalog: resolution.catalog,
        };
        let mut clock = || UNIX_EPOCH;
        deepgram_listen_passthrough_handler(logging_obj, &request, &services, &mut clock).await
    }

    // The calculator a served request prices a listen session with, borrowing this listener's catalog and rates.
    fn calculator<'services, Image>(
        &'services self,
        image: &'services Image,
        resolution: &'services ResolutionContext<'services>,
    ) -> CostCalculator<'services, CatalogLookup, Image, CatalogLookup> {
        CostCalculator {
            special: SpecialBorrows {
                lookup: &self.lookup,
                resolver: &self.lookup,
                image_policy: image,
                special_policy: &ProviderPricing,
                size_pattern: &self.pattern,
                database: &self.database,
                replicate_models: &self.models,
                replicate_price: Decimal::new(14, 4),
                discount: &self.discount,
                margin: &self.margin,
            },
            catalog: &self.lookup,
            inference: &self.lookup,
            counter: &Counter,
            price_policy: &ProviderPricing,
            resolution,
            tools: ToolBorrows {
                source: &self.lookup,
                policy: &self.tools,
                prices: &ToolPrices::PINNED,
            },
        }
    }
}

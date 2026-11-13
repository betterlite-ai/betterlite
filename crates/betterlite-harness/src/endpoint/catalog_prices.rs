use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_core::cost::call_price::policy::ProviderPricePolicy;
use betterlite_core::cost::call_price::{CallPriceEnv, CallPriceInput, call_price};
use betterlite_core::routes::CallType;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::clock::Clock;
use super::cost::{CostError, CostPerToken};

struct CatalogPolicy;

impl ProviderPricePolicy for CatalogPolicy {}

pub struct CatalogPrices {
    catalog: Arc<ModelCatalog>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    registry: JSONProviderRegistry,
    policies: ProviderPolicies,
    globals: GlobalCredentials,
    database: TimeZoneDatabase,
    clock: Arc<dyn Clock>,
}

impl CatalogPrices {
    #[must_use]
    pub fn bundled(clock: Arc<dyn Clock>) -> Self {
        let mut bundled = BundledCatalog::default();
        Self {
            catalog: bundled.get(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            database: TimeZoneDatabase::none(),
            clock,
        }
    }

    fn record(&self, key: &str) -> Option<PriceRecord> {
        let canonical = self.catalog.lookup.get(key)?;
        let entry = self.catalog.entries.get(canonical.as_ref())?;
        serde_json::from_value(serde_json::to_value(&entry.prices).ok()?).ok()
    }

    fn lookup(&self, model: &str, custom_llm_provider: Option<&str>) -> Option<PriceRecord> {
        if let Some(provider) = custom_llm_provider {
            let key = format!("{provider}/{model}");
            if let Some(record) = self.record(&key) {
                return Some(record);
            }
        }
        self.record(model)
    }

    fn price(&self, model: &str, input: &CallPriceInput<'_>) -> Result<Decimal, CostError> {
        let resolution = ResolutionContext {
            catalog: self.catalog.as_ref(),
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: self.policies.clone(),
            globals: self.globals.clone(),
        };
        let policy = CatalogPolicy;
        let costs = call_price(
            input,
            &CallPriceEnv {
                lookup: self,
                policy: &policy,
                resolution: &resolution,
                database: &self.database,
                moment: self.clock.now(),
            },
        )
        .map_err(|source| CostError::Unpriced {
            model: model.to_owned(),
            source: Box::new(source),
        })?;
        costs
            .prompt_cost
            .checked_add(costs.completion_cost)
            .ok_or_else(|| CostError::Overflow {
                model: model.to_owned(),
            })
    }
}

impl PriceLookup for CatalogPrices {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.lookup(model, custom_llm_provider)
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.record(key)
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

impl CostPerToken for CatalogPrices {
    fn cost_per_token(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<Decimal, CostError> {
        let prompt_tokens = count(input_tokens)?;
        let completion_tokens = count(output_tokens)?;
        let input = CallPriceInput {
            model: Some(model),
            prompt_tokens,
            completion_tokens,
            response_time_ms: None,
            custom_llm_provider: None,
            region_name: None,
            prompt_characters: None,
            completion_characters: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            custom_cost_per_token: None,
            custom_cost_per_second: None,
            number_of_queries: None,
            usage: None,
            rerank_billed_units: None,
            call_type: CallType::Completion,
            audio_seconds: Decimal::ZERO,
            service_tier: None,
            data_residency: None,
            vertex_location: None,
            vector_response: None,
            ocr_billing: None,
            search_hidden_params: None,
            request_model: None,
            custom_model_info: None,
            server_tool_projection: None,
        };
        self.price(model, &input)
    }
}

fn count(tokens: u64) -> Result<i64, CostError> {
    match i64::try_from(tokens) {
        Ok(count) => Ok(count),
        Err(_error) => Err(CostError::CountOverflow { tokens }),
    }
}

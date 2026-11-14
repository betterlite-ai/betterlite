use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{
    ModelCostMap, ModelCostMapError, NameNormalizers,
};
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, MetadataRequest, get_model_info_helper,
};
use betterlite_models::off_peak::schema::OffPeakBlockEntry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::tiered_pricing::Tier;
use futures_util::FutureExt as _;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;

const CACHE_READ_KEY: &str = "cache_read_input_token_cost";

#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("model cost map: {0}")]
    Map(#[from] ModelCostMapError),
    #[error("model catalog: {0}")]
    Catalog(#[from] CatalogError),
}

// The resolver reads no Hugging Face config here, so every fetch is refused and the metadata stays soft.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("cost lookups fetch no Hugging Face config")]
pub struct NoHfConfig;

impl ModelInfoClient for NoHfConfig {
    type Error = Self;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self>> + Send {
        ready(Err(Self))
    }
}

// Every outside value the model-info resolver reads, passed in so a lookup never reads the process environment.
pub struct LookupSettings {
    pub known_model_names: BTreeSet<String>,
    pub custom_providers: BTreeSet<String>,
    pub environment: BTreeMap<String, String>,
    pub registry: JSONProviderRegistry,
    pub policies: ProviderPolicies,
    pub globals: GlobalCredentials,
    pub normalizers: NameNormalizers,
}

// The B193 price map plus the B205 resolver: effective lookups resolve a name, raw lookups read one exact key.
pub struct CatalogLookup {
    map: ModelCostMap,
    catalog: ModelCatalog,
    settings: LookupSettings,
    hf: HfConfigClient<NoHfConfig>,
    keys: Mutex<ModelCostKeys>,
}

#[derive(Deserialize)]
struct Tables<'raw> {
    #[serde(borrow, default)]
    tiered_pricing: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    off_peak_pricing: Option<&'raw RawValue>,
}

impl CatalogLookup {
    pub fn new(model_cost: &[u8], settings: LookupSettings) -> Result<Self, LookupError> {
        Ok(Self {
            map: ModelCostMap::parse(model_cost)?,
            catalog: ModelCatalog::parse(model_cost, UnknownFields::Warn)?,
            settings,
            hf: HfConfigClient {
                client: NoHfConfig,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            keys: Mutex::new(ModelCostKeys::default()),
        })
    }

    pub fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.settings.known_model_names,
            custom_providers: &self.settings.custom_providers,
            environment: &self.settings.environment,
            registry: self.settings.registry.clone(),
            policies: self.settings.policies.clone(),
            globals: self.settings.globals.clone(),
        }
    }

    // The resolver is async only for its dynamic sources; with none installed it never waits.
    pub(super) fn resolved(&self, model: &str, provider: Option<&str>) -> Option<Box<RawValue>> {
        let resolution = self.resolution();
        let environment = MetadataEnvironment {
            model_cost: &self.map.entries,
            vertex: &self.map.vertex,
            generalizations: &self.catalog.generalizations,
            resolution: &resolution,
            normalizers: self.settings.normalizers,
        };
        let request = MetadataRequest {
            model,
            custom_llm_provider: provider,
            api_base: None,
            api_key: None,
        };
        let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
        get_model_info_helper(
            &request,
            &environment,
            &mut keys,
            &NoDynamicMetadata,
            &self.hf,
        )
        .now_or_never()?
        .ok()
    }
}

impl PriceLookup for CatalogLookup {
    fn effective_model_info(&self, model: &str, provider: Option<&str>) -> Option<PriceRecord> {
        serde_json::from_str(self.resolved(model, provider)?.get()).ok()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        serde_json::from_str(self.map.entries.get(key)?.get()).ok()
    }

    fn effective_priced_model(&self, model: &str, provider: Option<&str>) -> Option<PricedModel> {
        let raw = self.resolved(model, provider)?;
        let record = serde_json::from_str(raw.get()).ok()?;
        let tables: Tables<'_> = serde_json::from_str(raw.get()).ok()?;
        let tiered_pricing = tables
            .tiered_pricing
            .and_then(|tiers| serde_json::from_str::<Vec<Tier>>(tiers.get()).ok())
            .unwrap_or_default();
        let off_peak_pricing = tables
            .off_peak_pricing
            .and_then(|block| serde_json::from_str::<OffPeakBlockEntry>(block.get()).ok())
            .and_then(|entry| entry.as_block().cloned());
        let off_peak_cache_read_present = tables
            .off_peak_pricing
            .and_then(|block| {
                serde_json::from_str::<BTreeMap<String, IgnoredAny>>(block.get()).ok()
            })
            .is_some_and(|block| block.contains_key(CACHE_READ_KEY));
        Some(PricedModel {
            record,
            tiered_pricing,
            off_peak_pricing,
            off_peak_cache_read_present,
        })
    }
}

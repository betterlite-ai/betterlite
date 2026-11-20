use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_core::cost::completion::lookup::CatalogLookup;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};
use betterlite_models::off_peak::schema::{OffPeakBlock, OffPeakBlockEntry};
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::resolution::{ResolutionRequest, resolve};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use betterlite_models::tiered_pricing::{Scalar, Tier};
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_model_info::cache::DiscoveryCache;
use betterlite_router::deployment_model_info::deployment_info::get_deployment_model_info;
use betterlite_router::deployment_model_info::discovery::CatalogSnapshot;
use betterlite_router::deployment_model_info::metadata::{ModelMetadataSource, RouterMetadata};

const CACHE_READ_KEY: &str = "cache_read_input_token_cost";

#[derive(Debug, thiserror::Error)]
enum DeploymentPriceError {
    #[error("a deployment price record could not be decoded: {0}")]
    Decode(#[from] serde_json::Error),
}

// The per-deployment price overrides B845 reads, beside the public lookup its fallback goes through.
pub struct DeploymentPricing {
    catalog: Arc<DeploymentCatalog>,
    discovery: DiscoveryCache,
    registry: ModelRegistry,
}

impl Default for DeploymentPricing {
    fn default() -> Self {
        Self {
            catalog: Arc::new(DeploymentCatalog::default()),
            discovery: DiscoveryCache::for_deployments(0),
            registry: ModelRegistry::default(),
        }
    }
}

impl DeploymentPricing {
    async fn deployment_info(
        &self,
        lookup: &CatalogLookup,
        model_id: &str,
        model_name: &str,
    ) -> Option<JsonObject> {
        let resolution = lookup.resolution();
        let source = PublicModelInfo { lookup };
        let snapshot = CatalogSnapshot {
            catalog: &self.catalog,
        };
        let metadata = RouterMetadata {
            snapshot: &snapshot,
            discovery: &self.discovery,
            registry: &self.registry,
            resolution: &resolution,
            source: &source,
        };
        let mut keys = ModelCostKeys::default();
        get_deployment_model_info(&metadata, model_id, model_name, &mut keys).await
    }

    // B845: the deployment's own entry over the built-in one for the model name it serves.
    async fn price(
        &self,
        lookup: &CatalogLookup,
        model_id: &str,
        model_name: &str,
    ) -> Option<PricedModel> {
        let Some(info) = self.deployment_info(lookup, model_id, model_name).await else {
            tracing::debug!(
                model_id,
                model = model_name,
                "savings: no deployment pricing"
            );
            return None;
        };
        let public = lookup.effective_priced_model(model_name, None);
        match priced_from_info(&info, public.as_ref()) {
            Ok(priced) => Some(priced),
            Err(error) => {
                tracing::debug!(
                    model_id,
                    model = model_name,
                    %error,
                    "savings: the deployment pricing is unreadable"
                );
                None
            }
        }
    }
}

// The savings side of the cost resource: B174 identities, its bound public lookup and a proxy's deployment prices.
pub struct Context<'state> {
    lookup: &'state CatalogLookup,
    deployments: Option<DeploymentPricing>,
}

impl<'state> Context<'state> {
    #[must_use]
    pub const fn new(
        lookup: &'state CatalogLookup,
        deployments: Option<DeploymentPricing>,
    ) -> Self {
        Self {
            lookup,
            deployments,
        }
    }
}

impl SavingsPricingContext for Context<'_> {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        let resolution = self.lookup.resolution();
        let request = ResolutionRequest {
            model,
            custom_llm_provider,
            api_base: None,
            api_key: None,
        };
        let resolved = resolve(&request, &resolution).ok()?;
        Some(ModelIdentity {
            model: resolved.model,
            provider: resolved.provider.slug().to_owned(),
        })
    }

    async fn deployment_price(
        &self,
        deployment_id: Option<&str>,
        model: &str,
    ) -> Option<PricedModel> {
        let deployment = deployment_id?;
        let deployments = self.deployments.as_ref()?;
        deployments.price(self.lookup, deployment, model).await
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        self.lookup
    }
}

// What B845 calls litellm.get_model_info for a model name: the public entry's scalar prices.
struct PublicModelInfo<'state> {
    lookup: &'state CatalogLookup,
}

impl ModelMetadataSource for PublicModelInfo<'_> {
    type Error = Infallible;

    fn get_model_info(
        &self,
        model: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Infallible>> + Send {
        ready(Ok(self
            .lookup
            .effective_priced_model(model, None)
            .map(|priced| record_to_json(&priced.record))))
    }
}

fn record_to_json(record: &PriceRecord) -> JsonObject {
    record
        .iter()
        .filter_map(|(key, value)| scalar_to_json(value).map(|json| (key.clone(), json)))
        .collect()
}

fn scalar_to_json(value: &Scalar) -> Option<Json> {
    match value {
        Scalar::Null => Some(Json::Null),
        Scalar::Bool(flag) => Some(Json::Bool(*flag)),
        Scalar::Number(number) => Some(Json::Number(number.clone())),
        Scalar::Text(text) => Some(Json::Text(text.clone())),
        Scalar::Array(_) | Scalar::Object(_) => None,
    }
}

fn priced_from_info(
    info: &JsonObject,
    public: Option<&PricedModel>,
) -> Result<PricedModel, DeploymentPriceError> {
    let serialized = serde_json::to_string(info)?;
    let mut record: PriceRecord = serde_json::from_str(&serialized)?;
    if let Some(public_pricing) = public {
        for (key, value) in &public_pricing.record {
            record.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    let (tiered_pricing, off_peak_pricing, off_peak_cache_read_present) = tables(info, public);
    Ok(PricedModel {
        record,
        tiered_pricing,
        off_peak_pricing,
        off_peak_cache_read_present,
    })
}

// B845 merges custom fields over the built-in entry, so a table the custom entry carries wins.
fn tables(
    info: &JsonObject,
    public: Option<&PricedModel>,
) -> (Vec<Tier>, Option<OffPeakBlock>, bool) {
    let tiered = tiered_table(info, public);
    let (off_peak, cache_read_present) = off_peak_table(info, public);
    (tiered, off_peak, cache_read_present)
}

fn tiered_table(info: &JsonObject, public: Option<&PricedModel>) -> Vec<Tier> {
    let Some(value) = info.get("tiered_pricing") else {
        return public.map_or_else(Vec::new, |priced| priced.tiered_pricing.clone());
    };
    let serialized = serde_json::to_string(value);
    serialized
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn off_peak_table(info: &JsonObject, public: Option<&PricedModel>) -> (Option<OffPeakBlock>, bool) {
    let Some(value) = info.get("off_peak_pricing") else {
        return public.map_or((None, false), |priced| {
            (
                priced.off_peak_pricing.clone(),
                priced.off_peak_cache_read_present,
            )
        });
    };
    let serialized = serde_json::to_string(value);
    let block = serialized
        .ok()
        .and_then(|text| serde_json::from_str::<OffPeakBlockEntry>(&text).ok())
        .and_then(|entry| entry.as_block().cloned());
    (
        block,
        matches!(value, Json::Object(fields) if fields.contains_key(CACHE_READ_KEY)),
    )
}

mod tests;

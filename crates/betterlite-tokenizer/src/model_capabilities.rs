use std::sync::Arc;

use betterlite_context::bedrock_model::regions::AmazonBedrockGlobalConfig;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::endpoints::{EndpointRegistry, EndpointSupport};
use betterlite_models::lookup::info::{
    ModelId, ModelInfo, ModelInfoError, ModelQuery, PtuBounds, resolved_id, validated_ptu,
};
use betterlite_models::lookup::resolve::{BedrockNames, find_info};

use crate::count::Counter;
use crate::count::error::CountError;
use crate::count::wire::TokenRequestWire;

mod tests;

pub const BEDROCK_NAMES: BedrockNames = BedrockNames {
    base_model: get_bedrock_base_model,
    strip_routing_prefix: strip_bedrock_routing_prefix,
    strip_mantle_region,
};

// split_mantle_region_prefix: a leading Bedrock region segment goes, any other model stays.
fn strip_mantle_region(model: &str) -> String {
    match model.split_once('/') {
        Some((head, tail)) if AmazonBedrockGlobalConfig.get_all_regions().contains(&head) => {
            tail.to_owned()
        }
        _ => model.to_owned(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteStatus {
    pub endpoint: String,
    pub catalog_support: EndpointSupport,
    // catalog metadata never proves an adapter exists; provider adapters are installed by their own layer
    pub adapter_installed: bool,
}

pub struct ModelCapabilities {
    catalog: Arc<ModelCatalog>,
    counter: Arc<Counter>,
    registry: Arc<EndpointRegistry>,
}

impl ModelCapabilities {
    #[must_use]
    pub const fn new(
        catalog: Arc<ModelCatalog>,
        counter: Arc<Counter>,
        registry: Arc<EndpointRegistry>,
    ) -> Self {
        Self {
            catalog,
            counter,
            registry,
        }
    }

    pub fn model_info(
        &self,
        query: &ModelQuery<'_>,
        id: Option<ModelId<'_>>,
        ptu: &PtuBounds,
    ) -> Result<ModelInfo, ModelInfoError> {
        let Some(resolved) = find_info(&self.catalog, &BEDROCK_NAMES, query)? else {
            return Err(ModelInfoError::unmapped(query));
        };
        let validated = validated_ptu(ptu)?;
        Ok(ModelInfo {
            id: resolved_id(id),
            key: resolved.key,
            model: query.model.to_owned(),
            provider: resolved.provider,
            mode: resolved.mode,
            limits: resolved.limits,
            capabilities: resolved.capabilities,
            ptu_count: validated.count,
            cost_per_ptu_per_hour: validated.cost_per_ptu_per_hour,
            ptu_effective_from: validated.effective_from,
            ptu_effective_to: validated.effective_to,
        })
    }

    pub fn route_statuses(
        &self,
        query: &ModelQuery<'_>,
    ) -> Result<Vec<RouteStatus>, ModelInfoError> {
        let Some(resolved) = find_info(&self.catalog, &BEDROCK_NAMES, query)? else {
            return Err(ModelInfoError::unmapped(query));
        };
        let Some(provider) = resolved.provider else {
            return Ok(Vec::new());
        };
        let Some(capabilities) = self.registry.providers.get(&provider) else {
            return Ok(Vec::new());
        };
        Ok(capabilities
            .endpoints
            .iter()
            .map(|(endpoint, supported)| RouteStatus {
                endpoint: endpoint.clone(),
                catalog_support: if *supported {
                    EndpointSupport::Supported
                } else {
                    EndpointSupport::Unsupported
                },
                adapter_installed: false,
            })
            .collect())
    }

    pub async fn count_tokens(&self, wire: &TokenRequestWire) -> Result<usize, CountError> {
        let selected = wire.selected()?;
        let request = wire.request(selected.as_ref());
        self.counter.count(&request).await
    }
}

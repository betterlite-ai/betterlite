use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::endpoints::{EndpointRegistry, EndpointSupport};
use betterlite_models::lookup::capabilities::CapabilityView;
use betterlite_models::lookup::info::{
    ModelId, ModelInfo, ModelInfoError, ModelQuery, PtuBounds, declared_provider, find_entry,
    resolved_id, validated_ptu,
};
use betterlite_models::lookup::limits::ModelLimits;

use crate::count::Counter;
use crate::count::error::CountError;
use crate::count::wire::TokenRequestWire;

mod tests;

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
        let Some(entry) = find_entry(&self.catalog, query) else {
            return Err(ModelInfoError::unmapped(query));
        };
        let validated = validated_ptu(ptu)?;
        Ok(ModelInfo {
            id: resolved_id(id),
            key: entry.name.clone(),
            model: query.model.to_owned(),
            provider: declared_provider(entry, query),
            mode: entry.fields.metadata.mode,
            limits: ModelLimits::from_fields(&entry.fields.metadata),
            capabilities: CapabilityView::from_fields(&entry.fields.capabilities),
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
        let Some(entry) = find_entry(&self.catalog, query) else {
            return Err(ModelInfoError::unmapped(query));
        };
        let Some(provider) = declared_provider(entry, query) else {
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

use std::collections::{BTreeMap, BTreeSet};

use betterlite_bedrock::canvas_edits::eligibility::supports_nova_canvas_edit;
use betterlite_bedrock::canvas_edits::error::CanvasEditError;
use betterlite_bedrock::canvas_edits::selection::{ImageEditConfig, image_edit_config_for_model};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;

// The typed catalog stays empty: eligibility reads the raw model_cost map the case supplies.
pub(super) struct Catalog {
    typed: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Catalog {
    pub(super) fn new() -> Result<Self, CatalogError> {
        Ok(Self {
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)?,
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        })
    }

    pub(super) fn eligible(&self, model: &str, model_cost: &ParamMap) -> bool {
        let context = self.context();
        supports_nova_canvas_edit(model, model_cost, &context, &mut ModelCostKeys::default())
    }

    pub(super) fn selected(
        &self,
        model: &str,
        model_cost: &ParamMap,
    ) -> Result<ImageEditConfig, CanvasEditError> {
        let context = self.context();
        image_edit_config_for_model(model, model_cost, &context, &mut ModelCostKeys::default())
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

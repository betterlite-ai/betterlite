use betterlite_models::catalog::ModelCatalog;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest, ResolvedModel};

use crate::health_probe_settings::model_info::mode_name;

// get_llm_provider behind the entry: the caller injects the resolver with its catalog, provider sets and defaults
pub trait ProviderResolver: Send + Sync {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError>;
}

// LiteLLM's `model in litellm.model_cost` plus the entry's `mode` field, which may be absent
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogMode {
    NotListed,
    NoMode,
    Mode(String),
}

pub trait CatalogModes: Send + Sync {
    fn catalog_mode(&self, model: &str) -> CatalogMode;
}

impl CatalogModes for ModelCatalog {
    fn catalog_mode(&self, model: &str) -> CatalogMode {
        let Some(metadata) = self.metadata(model) else {
            return CatalogMode::NotListed;
        };
        metadata.mode.map_or(CatalogMode::NoMode, |mode| {
            CatalogMode::Mode(mode_name(mode).to_owned())
        })
    }
}

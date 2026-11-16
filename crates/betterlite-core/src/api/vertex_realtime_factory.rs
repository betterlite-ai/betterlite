use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;
use secrecy::SecretString;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;
use crate::api::vertex_chirp::config::VertexChirpRealtimeConfig;
use crate::api::vertex_chirp::error::ChirpAdapterError;
use crate::api::vertex_chirp::factory::ChirpBackendFactory;
use crate::api::vertex_chirp::ids::ChirpIds;
use crate::api::vertex_chirp::settings::ChirpSettings;
use crate::api::vertex_chirp::target::AccessTokenResolver;
use crate::api::vertex_chirp_protocol::is_vertex_speech_to_text_model;
use crate::api::vertex_location::{
    VertexLocationError, validate_vertex_location, validate_vertex_project,
};

mod tests;

pub trait VertexRegionPolicy: Send + Sync {
    fn region_for(&self, catalog: &ModelCatalog, model: &str, configured: Option<&str>) -> String;
}

pub struct VertexRealtimeInput<'input> {
    pub model: &'input str,
    pub project: &'input str,
    pub location: Option<&'input str>,
    pub access_token: SecretString,
    pub resolve_access_token: Arc<dyn AccessTokenResolver>,
    pub catalog: Arc<ModelCatalog>,
    pub limits: ApiLimits,
    pub ids: Box<dyn ChirpIds>,
}

pub struct VertexLiveTarget {
    pub model: String,
    pub project: String,
    pub region: String,
    pub access_token: SecretString,
}

// both variants are boxed to stay within 3x of each other, the limit variant_size_differences enforces
pub enum VertexRealtimeConfig<Factory> {
    Chirp(Box<VertexChirpRealtimeConfig<Factory>>),
    Live(Box<VertexLiveTarget>),
}

#[derive(Debug, Error)]
pub enum VertexRealtimeError {
    #[error(transparent)]
    Chirp(#[from] ChirpAdapterError),
    #[error(transparent)]
    Target(#[from] VertexLocationError),
}

pub fn vertex_realtime_config<Factory>(
    input: VertexRealtimeInput<'_>,
    regions: &dyn VertexRegionPolicy,
    factory: Factory,
) -> Result<VertexRealtimeConfig<Factory>, VertexRealtimeError>
where
    Factory: ChirpBackendFactory,
{
    if is_vertex_speech_to_text_model(&input.catalog, input.model) {
        let settings = ChirpSettings {
            project: input.project.to_owned(),
            location: input.location.map(str::to_owned),
            catalog: input.catalog,
            limits: input.limits,
            ids: input.ids,
        };
        let config = VertexChirpRealtimeConfig::new(settings, input.resolve_access_token, factory)?;
        return Ok(VertexRealtimeConfig::Chirp(Box::new(config)));
    }
    let project = validate_vertex_project(input.project)?.to_owned();
    let configured = input.location.filter(|location| !location.is_empty());
    let selected = regions.region_for(&input.catalog, input.model, configured);
    let region = validate_vertex_location(Some(selected.as_str()))?.to_owned();
    Ok(VertexRealtimeConfig::Live(Box::new(VertexLiveTarget {
        model: input.model.to_owned(),
        project,
        region,
        access_token: input.access_token,
    })))
}

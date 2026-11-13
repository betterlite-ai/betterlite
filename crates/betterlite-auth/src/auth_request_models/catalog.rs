use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::Arc;

use thiserror::Error;

use super::RequestedModels;
use super::model::get_model_from_request;
use super::sources::RequestSnapshot;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("the deployment is stored in an invalid format")]
    InvalidDeployment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentRecord {
    pub model_name: String,
    pub custom_llm_provider: Option<String>,
    pub model: Option<String>,
    pub team_id: Option<String>,
    pub deployment_id: Option<String>,
}

pub trait ModelCatalog: Debug + Send + Sync {
    fn resolve_model_name_from_model_id(
        &self,
        model_id: &str,
    ) -> Result<Option<String>, CatalogError>;

    fn deployment(&self, deployment_id: &str) -> Result<Option<DeploymentRecord>, CatalogError>;

    fn model_names(&self) -> Result<BTreeSet<String>, CatalogError>;

    fn model_list(&self) -> Result<Vec<DeploymentRecord>, CatalogError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyCatalog;

impl ModelCatalog for EmptyCatalog {
    fn resolve_model_name_from_model_id(
        &self,
        _model_id: &str,
    ) -> Result<Option<String>, CatalogError> {
        Ok(None)
    }

    fn deployment(&self, _deployment_id: &str) -> Result<Option<DeploymentRecord>, CatalogError> {
        Ok(None)
    }

    fn model_names(&self) -> Result<BTreeSet<String>, CatalogError> {
        Ok(BTreeSet::new())
    }

    fn model_list(&self) -> Result<Vec<DeploymentRecord>, CatalogError> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManagedIdError {
    #[error("a managed resource id decoder failed")]
    Decoder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdField {
    Video,
    Character,
    Other,
}

impl ManagedIdField {
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "video_id" => Self::Video,
            "character_id" => Self::Character,
            _ => Self::Other,
        }
    }
}

pub trait ManagedIdDecoders: Debug + Send + Sync {
    fn legacy_file_batch(
        &self,
        resource_id: &str,
        catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError>;

    fn unified(
        &self,
        resource_id: &str,
        catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError>;

    fn video_character(
        &self,
        resource_id: &str,
        field: ManagedIdField,
        catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError>;
}

pub struct Collaborators<'collaborators> {
    pub catalog: Option<&'collaborators dyn ModelCatalog>,
    pub decoders: &'collaborators dyn ManagedIdDecoders,
}

#[derive(Debug)]
pub struct RequestModelResolver {
    pub catalog: Arc<dyn ModelCatalog>,
    pub decoders: Arc<dyn ManagedIdDecoders>,
}

impl RequestModelResolver {
    pub fn resolve(&self, request: &RequestSnapshot<'_>) -> Result<RequestedModels, CatalogError> {
        let collaborators = Collaborators {
            catalog: Some(self.catalog.as_ref()),
            decoders: self.decoders.as_ref(),
        };
        get_model_from_request(request, &collaborators)
    }
}

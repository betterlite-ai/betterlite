use std::io;

use betterlite_models::model_metadata_resolution::names::ModelCostMapError;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::model_group_info::error::GroupInfoError;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;

use super::sources::ProviderUnavailable;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Group(#[from] GroupInfoError<ProviderUnavailable>),
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    CostMap(#[from] ModelCostMapError),
    #[error(transparent)]
    Runtime(#[from] io::Error),
    #[error("deployment_info.{0} is not an object")]
    Metadata(String),
    #[error("parameters.{0} is not a list of texts")]
    Parameters(String),
}

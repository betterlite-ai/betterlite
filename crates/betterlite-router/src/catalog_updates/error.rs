use crate::deployment_identity::error::{DeploymentError, InvalidDeployment};
use crate::wildcard_catalog::error::PatternCatalogError;

// What stops a catalog update; the published catalog stays as it was when one of these comes back.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError<Fetch> {
    #[error(transparent)]
    Invalid(#[from] InvalidDeployment),
    #[error(transparent)]
    Deployment(#[from] DeploymentError<Fetch>),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
}

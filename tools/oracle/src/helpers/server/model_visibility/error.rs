use std::io;

use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum HealthError {
    #[error("the health cache is unreachable")]
    Unreachable,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Lookup(#[from] LookupError),
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("the health cache cannot answer: {0}")]
    Health(#[from] HealthError),
    #[error(transparent)]
    Runtime(#[from] io::Error),
}

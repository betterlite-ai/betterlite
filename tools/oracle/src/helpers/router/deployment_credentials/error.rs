use betterlite_router::deployment_credentials::error::CredentialError;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;

// The ways a case fails before an answer: a refused credential lookup, a row the Router will not build, or a missing argument.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
    #[error("arguments.{0} is required")]
    Argument(&'static str),
}

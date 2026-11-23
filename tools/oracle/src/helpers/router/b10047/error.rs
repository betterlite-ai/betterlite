use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::model_group_lookups::error::AccessGroupError;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;

// The ways a case fails before an answer: the map's own refusal, or a catalog that will not build.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    AccessGroups(#[from] AccessGroupError),
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
}

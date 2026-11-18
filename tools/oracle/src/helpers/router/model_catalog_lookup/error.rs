use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::routing_groups::error::RoutingGroupError;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;

// The ways a case fails before an answer: a lookup refusal, a catalog that will not build, or a missing argument.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Lookup(#[from] LookupError),
    #[error(transparent)]
    RoutingGroup(#[from] RoutingGroupError),
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
    #[error("arguments.{0} is required")]
    Argument(&'static str),
}

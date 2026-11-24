use std::convert::Infallible;
use std::io;

use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::model_group_lookups::error::AccessGroupError;
use betterlite_router::model_group_usage::error::GroupUsageError;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;

// The one failure the scripted counter read raises.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("the scripted counter read failed")]
pub(crate) struct CacheFailure;

// Every way a case can refuse to answer, with the failure class basedpyright-visible code raises.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Runtime(#[from] io::Error),
    #[error(transparent)]
    Counter(#[from] CacheFailure),
    #[error(transparent)]
    Usage(#[from] GroupUsageError<Infallible, CacheFailure>),
    #[error(transparent)]
    AccessGroup(#[from] AccessGroupError),
    #[error("a response the case cannot carry")]
    UnsupportedResponse,
    #[error("deployment {index} is outside the case's model list")]
    Candidate { index: usize },
    #[error("the {function} function needs a model_group")]
    MissingGroup { function: &'static str },
    #[error("now must be a whole number of seconds")]
    Clock,
}

use thiserror::Error;

use crate::affinity_claims::error::AffinityError;

// the check's failures: a request field it reads has the wrong type, or the affinity cache refuses the read
#[derive(Debug, Error)]
pub enum DeploymentAffinityError {
    #[error("the request field {field} is not a string")]
    RequestNotText { field: &'static str },
    #[error(transparent)]
    Affinity(#[from] AffinityError),
}

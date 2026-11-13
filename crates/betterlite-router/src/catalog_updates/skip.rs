use std::error::Error;

use super::error::CatalogError;
use crate::deployment_identity::construct::ignore_or_raise;
use crate::deployment_identity::error::DeploymentError;

// The except branch of _create_deployment for a failure of the add step: raise it, or log it and say whether the retry queue takes it.
pub(super) fn skip_or_raise<Fetch>(
    ignore_invalid_deployments: bool,
    error: CatalogError<Fetch>,
) -> Result<bool, CatalogError<Fetch>>
where
    Fetch: Error + 'static,
{
    let deployment = match error {
        CatalogError::Invalid(invalid) => DeploymentError::Invalid(invalid),
        CatalogError::Deployment(deployment) => deployment,
        CatalogError::Pattern(pattern) if ignore_invalid_deployments => {
            tracing::error!(
                "Error creating deployment: {pattern}, ignoring and continuing with other deployments."
            );
            return Ok(false);
        }
        CatalogError::Pattern(pattern) => return Err(CatalogError::Pattern(pattern)),
    };
    ignore_or_raise(ignore_invalid_deployments, deployment).map_err(CatalogError::Deployment)
}

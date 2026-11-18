use axum::extract::Extension;
use betterlite_config::credentials::CredentialValues;

use crate::auth::auth_identity::AuthContext;

use super::error::CredentialError;
use super::lookup::DeploymentLookup;

pub(super) fn actor(identity: Option<&Extension<AuthContext>>) -> Option<String> {
    let Extension(context) = identity?;
    let user = context.principal.user.as_ref()?;
    Some(user.id.clone())
}

pub(super) fn deployment_values(
    lookup: Option<&dyn DeploymentLookup>,
    model_id: &str,
) -> Result<CredentialValues, CredentialError> {
    let Some(source) = lookup else {
        return Err(CredentialError::DeploymentRouterMissing);
    };
    let deployment = source
        .deployment_credentials(model_id)
        .ok_or(CredentialError::ModelNotFound)?;
    Ok(deployment.credential_values)
}

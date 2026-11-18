use axum::extract::Extension;
use betterlite_db::identity_rows::Payload;

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
) -> Result<Payload, CredentialError> {
    let Some(lookup) = lookup else {
        return Err(CredentialError::DeploymentRouterMissing);
    };
    let deployment = lookup
        .deployment_credentials(model_id)
        .ok_or(CredentialError::ModelNotFound)?;
    Ok(deployment.credential_values)
}

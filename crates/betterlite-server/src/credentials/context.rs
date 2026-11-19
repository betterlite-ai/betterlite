use axum::extract::Extension;
use betterlite_config::credentials::CredentialValues;

use crate::auth::auth_identity::AuthContext;

use super::error::CredentialError;
use super::lookup::{DeploymentCredentials, DeploymentLookup};

pub(super) fn actor(identity: Option<&Extension<AuthContext>>) -> Option<String> {
    let Extension(context) = identity?;
    let user = context.principal.user.as_ref()?;
    Some(user.id.clone())
}

pub(super) fn resolve_deployment(
    lookup: Option<&dyn DeploymentLookup>,
    model_id: &str,
    router_missing: CredentialError,
) -> Result<DeploymentCredentials, CredentialError> {
    let Some(source) = lookup else {
        return Err(router_missing);
    };
    source
        .deployment_credentials(model_id)
        .ok_or(CredentialError::ModelNotFound)
}

pub(super) fn deployment_values(
    lookup: Option<&dyn DeploymentLookup>,
    model_id: &str,
) -> Result<CredentialValues, CredentialError> {
    let deployment =
        resolve_deployment(lookup, model_id, CredentialError::DeploymentRouterMissing)?;
    Ok(deployment.credential_values)
}

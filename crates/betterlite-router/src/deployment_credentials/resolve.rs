use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_credentials::error::CredentialError;
use crate::deployment_credentials::params::CredentialParams;
use crate::deployment_credentials::selection::{
    credential_deployment, deployment_by_id, is_deployment_blocked,
};

// The provider LiteLLM restores: an explicit custom_llm_provider wins even when empty, else the text before the first slash, else openai.
fn provider(params: &JsonObject, model: &str) -> String {
    match params.get("custom_llm_provider") {
        Some(Json::Text(declared)) => declared.clone(),
        _ => model
            .split_once('/')
            .map_or_else(|| "openai".to_owned(), |(prefix, _rest)| prefix.to_owned()),
    }
}

// LiteLLM reads litellm_credential_name as an optional string; an empty name is still a name.
fn credential_name(params: &JsonObject) -> Result<Option<&str>, CredentialError> {
    match params.get("litellm_credential_name") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name)),
        Some(_) => Err(CredentialError::FieldType {
            field: "litellm_credential_name".to_owned(),
            expected: "a string",
        }),
    }
}

// Router.get_deployment_credentials: the exact id only, the same blocked test, and the credential projection.
pub fn deployment_credentials(
    catalog: &DeploymentCatalog,
    model_id: &str,
) -> Result<Option<CredentialParams>, CredentialError> {
    let Some(selected) = deployment_by_id(catalog, model_id)? else {
        return Ok(None);
    };
    if is_deployment_blocked(&selected.deployment) {
        return Ok(None);
    }
    CredentialParams::project(&selected.deployment.litellm_params).map(Some)
}

// Router.get_deployment_credentials_with_provider: the selection, the projection, the named overlay, then the restored model and provider.
pub fn credentials_with_provider(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
    credential_list: &CredentialSnapshot,
) -> Result<Option<CredentialParams>, CredentialError> {
    let Some(selected) = credential_deployment(catalog, model_id, team_id)? else {
        return Ok(None);
    };
    let model = selected.model;
    let params = &selected.deployment.litellm_params;
    let resolved = provider(params, &model);
    let mut credentials = CredentialParams::project(params)?;
    if let Some(name) = credential_name(params)? {
        let values = credential_list.values(name);
        if values.is_empty() {
            tracing::warn!("Credential '{name}' not found in credential_list");
        }
        credentials.fields.extend(values);
        credentials.fields.shift_remove("litellm_credential_name");
    }
    // these two override a same-named field a credential record supplied
    credentials
        .fields
        .insert("model".to_owned(), Json::Text(model));
    credentials
        .fields
        .insert("custom_llm_provider".to_owned(), Json::Text(resolved));
    Ok(Some(credentials))
}

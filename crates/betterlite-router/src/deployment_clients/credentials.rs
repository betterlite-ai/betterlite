use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::{ExposeSecret as _, SecretString};

use crate::deployment_identity::error::InvalidDeployment;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_identity::stable_id::generate_model_id;

pub const CLIENTSIDE_CREDENTIAL_KEYS: [&str; 3] = ["api_key", "api_base", "base_url"];

// The clientside part of a request: the key is used, the destinations are only markers for B052's 400.
#[derive(Clone, Debug, Default)]
pub struct ClientsideCredentials {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub base_url: Option<String>,
}

#[must_use]
pub fn is_clientside_credential(request: &JsonObject) -> bool {
    CLIENTSIDE_CREDENTIAL_KEYS
        .iter()
        .any(|key| request.contains_key(*key))
}

// the request key replaces the configured one; a request destination is never written, so the configured base stays
#[must_use]
pub fn dynamic_litellm_params(
    mut litellm_params: JsonObject,
    request: &ClientsideCredentials,
) -> JsonObject {
    if let Some(key) = &request.api_key {
        litellm_params.insert(
            "api_key".to_owned(),
            Json::Text(key.expose_secret().to_owned()),
        );
    }
    litellm_params
}

// The key stays a SecretString beside the spec, since Debug on the spec would print a litellm_params entry
#[derive(Clone, Debug)]
pub struct ClientsideDeployment {
    pub deployment: DeploymentSpec,
    pub api_key: Option<SecretString>,
}

pub fn clientside_deployment(
    deployment: &DeploymentSpec,
    model_group: &str,
    request: &ClientsideCredentials,
) -> Result<ClientsideDeployment, InvalidDeployment> {
    let hashed = dynamic_litellm_params(deployment.litellm_params.clone(), request);
    let mut params = deployment.litellm_params.clone();
    if request.api_key.is_some() {
        params.shift_remove("api_key");
    }
    let original_id = deployment
        .model_info
        .get("id")
        .cloned()
        .unwrap_or(Json::Null);
    let mut model_info = deployment.model_info.clone();
    model_info.insert(
        "id".to_owned(),
        Json::Text(generate_model_id(model_group, &hashed)),
    );
    model_info.insert("original_model_id".to_owned(), original_id);
    let spec = DeploymentSpec::new(
        model_group.to_owned(),
        params,
        model_info,
        deployment.extra.clone(),
        String::new,
    )?;
    Ok(ClientsideDeployment {
        deployment: spec,
        api_key: request.api_key.clone(),
    })
}

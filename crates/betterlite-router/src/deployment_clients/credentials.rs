use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::{ExposeSecret as _, SecretString};

use crate::deployment_identity::error::InvalidDeployment;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_identity::stable_id::generate_model_id;

// clientside_credential_keys: LiteLLM accepts these three from the request itself
pub const CLIENTSIDE_CREDENTIAL_KEYS: [&str; 3] = ["api_key", "api_base", "base_url"];

// The clientside part of a request: the key is used, the destinations are only markers for B052's 400.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClientsideCredentials {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub base_url: Option<String>,
}

// is_clientside_credential: presence alone, so an explicit null still counts the request as clientside
#[must_use]
pub fn is_clientside_credential(request: &JsonObject) -> bool {
    CLIENTSIDE_CREDENTIAL_KEYS
        .iter()
        .any(|key| request.contains_key(*key))
}

// get_dynamic_litellm_params: the request key replaces the configured one. A request destination is never written,
// because the only URL a client may use is the deployment's configured base.
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

// _handle_clientside_credential: a per-request deployment under a generated id, never registered with the router.
// The caller registers its pricing (B9710), exactly like the Python method does after building it.
pub fn clientside_deployment(
    deployment: &DeploymentSpec,
    model_group: &str,
    request: &ClientsideCredentials,
) -> Result<DeploymentSpec, InvalidDeployment> {
    let dynamic = dynamic_litellm_params(deployment.litellm_params.clone(), request);
    let original_id = deployment
        .model_info
        .get("id")
        .cloned()
        .unwrap_or(Json::Null);
    let mut model_info = deployment.model_info.clone();
    model_info.insert(
        "id".to_owned(),
        Json::Text(generate_model_id(model_group, &dynamic)),
    );
    model_info.insert("original_model_id".to_owned(), original_id);
    DeploymentSpec::new(
        model_group.to_owned(),
        dynamic,
        model_info,
        deployment.extra.clone(),
        String::new,
    )
}

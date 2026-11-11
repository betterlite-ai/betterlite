use betterlite_config::sections::mock_response::MockResponse;
use betterlite_config::sections::mock_response::selection::MockPermission;
use betterlite_config::types::{Config, ModelDeployment};
use betterlite_openai::chat::MockSettings;
use secrecy::ExposeSecret;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

use super::ChatDeployment;

// LiteLLM's generate_model_id hashes the model group and each litellm_params key and value, mock_response included
pub(super) fn mock_model_id(entry: &ModelDeployment, mock: &MockResponse) -> Option<String> {
    if entry.model_info.id.is_some() {
        return None;
    }
    let params = &entry.litellm_params;
    let api_key = params.api_key.as_ref().map(ExposeSecret::expose_secret);
    let fields: [(&str, Option<String>); 10] = [
        ("model", params.model.clone()),
        ("api_base", params.api_base.clone()),
        ("api_key", api_key.map(str::to_owned)),
        ("api_version", params.api_version.clone()),
        ("timeout", params.timeout.map(|value| value.to_string())),
        (
            "stream_timeout",
            params.stream_timeout.map(|value| value.to_string()),
        ),
        (
            "max_retries",
            params.max_retries.map(|value| value.to_string()),
        ),
        ("rpm", params.rpm.map(|value| value.to_string())),
        ("tpm", params.tpm.map(|value| value.to_string())),
        ("weight", params.weight.map(|value| value.to_string())),
    ];
    let mut hasher = Sha256::new();
    hasher.update(entry.model_name.as_bytes());
    for (name, value) in fields {
        if let Some(text) = value {
            hasher.update(name.as_bytes());
            hasher.update(text.as_bytes());
        }
    }
    hasher.update(b"mock_response");
    hasher.update(mock_id_value(mock).as_bytes());
    Some(hex::encode(hasher.finalize()))
}

// a string is hashed as given and a number vector as Python prints it; an object keeps betterlite's JSON text (deviation)
fn mock_id_value(mock: &MockResponse) -> String {
    match mock {
        MockResponse::Text(text) => text.clone(),
        MockResponse::Numbers(values) => {
            let parts: Vec<String> = values.iter().map(ToString::to_string).collect();
            format!("[{}]", parts.join(", "))
        }
        MockResponse::Object(_) => serde_json::to_string(mock).unwrap_or_default(),
    }
}

// LiteLLM's model id hashes litellm_params, mock_response included, so deployments that differ only in it stay distinct
pub(super) fn mock_settings(config: &Config, deployments: &mut [ChatDeployment]) -> MockSettings {
    let section = &config.sections.mock_response;
    let mut responses = BTreeMap::new();
    for (index, deployment) in deployments.iter_mut().enumerate() {
        let Some(response) = section.deployments.get(index).and_then(Option::as_ref) else {
            continue;
        };
        let entry = config.model_list.get(index);
        if let Some(id) = entry.and_then(|listed| mock_model_id(listed, response)) {
            deployment.model_id = id;
        }
        responses.insert(deployment.model_id.clone(), response.clone());
    }
    MockSettings {
        responses,
        usage: section.usage,
        // auth metadata is not wired yet, so a client mock field is always stripped until B052 and B057
        permission: MockPermission::default(),
    }
}

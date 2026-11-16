use axum::Json;
use betterlite_models::prompt_spec::{PromptInfo, PromptSpec};
use betterlite_prompts::registry::prompt_endpoint_contracts::records::Prompt;
use betterlite_prompts::registry::prompt_endpoint_contracts::rows::{PromptRowData, PromptRowJson};
use betterlite_prompts::registry::prompt_registry_helpers::DEFAULT_PROMPT_ENVIRONMENT;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MASKED_API_KEY: &str = "********";

#[derive(Debug, Error)]
#[error("invalid request body at {path}")]
pub(super) struct RequestBodyError {
    pub(super) path: String,
    #[source]
    pub(super) source: serde_json::Error,
}

#[derive(Deserialize)]
pub(super) struct EnvironmentQuery {
    pub(super) environment: Option<String>,
}

#[derive(Serialize)]
pub(super) struct DeletedPrompt {
    pub(super) message: String,
}

pub(super) fn decode_body<Body>(body: &[u8]) -> Result<Body, RequestBodyError>
where
    Body: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let decoded =
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| RequestBodyError {
            path: error.path().to_string(),
            source: error.into_inner(),
        })?;
    deserializer.end().map_err(|source| RequestBodyError {
        path: String::new(),
        source,
    })?;
    Ok(decoded)
}

// LiteLLM returns the stored key to the caller; the response carries a mask instead and only the database row keeps the key
pub(super) fn masked(mut spec: PromptSpec) -> Json<PromptSpec> {
    spec.litellm_params.api_key = spec
        .litellm_params
        .api_key
        .map(|_| SecretString::from(MASKED_API_KEY.to_owned()));
    Json(spec)
}

pub(super) fn new_row(
    prompt_id: &str,
    version: i64,
    environment: String,
    user_id: Option<String>,
    request: &Prompt,
) -> Result<PromptRowData, serde_json::Error> {
    let prompt_info = request.prompt_info.clone().unwrap_or_else(|| PromptInfo {
        prompt_type: "db".to_owned(),
        environment: DEFAULT_PROMPT_ENVIRONMENT.to_owned(),
        extra: IndexMap::new(),
    });
    Ok(PromptRowData {
        prompt_id: prompt_id.to_owned(),
        version,
        environment,
        created_by: user_id,
        litellm_params: Some(PromptRowJson::Text(serde_json::to_string(
            &request.litellm_params,
        )?)),
        prompt_info: Some(PromptRowJson::Text(serde_json::to_string(&prompt_info)?)),
        created_at: None,
        updated_at: None,
    })
}

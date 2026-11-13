use std::collections::BTreeMap;
use std::io;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::agent_launch::EnvVars;

const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
pub const OPENCODE_CONFIG_CONTENT_ENV: &str = "OPENCODE_CONFIG_CONTENT";
const OPENCODE_PROVIDER_ID: &str = "litellm";
const OPENCODE_PROVIDER_NAME: &str = "LiteLLM";
const OPENCODE_PROVIDER_NPM: &str = "@ai-sdk/openai-compatible";
const CHAT_MODES: [&str; 2] = ["chat", "responses"];
const LISTING_TIMEOUT: Duration = Duration::from_secs(10);
// a /v1/models row is a few hundred bytes, so 8 MiB covers tens of thousands of models
const MAX_LISTING_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ListedModel {
    pub id: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub max_input_tokens: Option<i64>,
    #[serde(default)]
    pub max_output_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct ModelListing {
    data: Vec<ListedModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSyncSkipped {
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSyncArgs {
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelSyncResult {
    Env(EnvVars),
    Args(ModelSyncArgs),
    Skipped(ModelSyncSkipped),
}

pub struct ModelListingRequest {
    pub url: String,
    pub authorization: SecretString,
    pub timeout: Duration,
    pub max_body_bytes: usize,
}

#[derive(Debug)]
pub struct ModelListingResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait ModelListingTransport {
    fn get(&self, request: &ModelListingRequest) -> io::Result<ModelListingResponse>;
}

#[derive(Serialize)]
struct OpenCodeLimit {
    context: i64,
    output: i64,
}

#[derive(Serialize)]
struct OpenCodeModel {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<OpenCodeLimit>,
}

#[derive(Serialize)]
struct OpenCodeProviderOptions {
    #[serde(rename = "baseURL")]
    base_url: String,
    #[serde(rename = "apiKey")]
    api_key: String,
}

#[derive(Serialize)]
struct OpenCodeProvider {
    npm: &'static str,
    name: &'static str,
    options: OpenCodeProviderOptions,
    models: BTreeMap<String, OpenCodeModel>,
}

#[derive(Serialize)]
struct OpenCodeConfig {
    provider: BTreeMap<String, OpenCodeProvider>,
}

#[must_use]
pub fn chat_models(models: &[ListedModel]) -> Vec<ListedModel> {
    models
        .iter()
        .filter(|model| {
            model
                .mode
                .as_deref()
                .is_none_or(|mode| CHAT_MODES.contains(&mode))
        })
        .cloned()
        .collect()
}

pub fn fetch_model_listing(
    base_url: &str,
    api_key: &SecretString,
    transport: &dyn ModelListingTransport,
) -> Result<Vec<ListedModel>, ModelSyncSkipped> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let key = api_key.expose_secret();
    let request = ModelListingRequest {
        url: url.clone(),
        authorization: SecretString::from(format!("Bearer {key}")),
        timeout: LISTING_TIMEOUT,
        max_body_bytes: MAX_LISTING_BODY_BYTES,
    };
    let response = match transport.get(&request) {
        Ok(response) => response,
        Err(source) => {
            return Err(ModelSyncSkipped {
                reason: format!("could not reach {url}: {source}"),
            });
        }
    };
    let status = response.status;
    if status != 200 {
        return Err(ModelSyncSkipped {
            reason: format!("{url} returned HTTP {status}"),
        });
    }
    if response.body.len() > MAX_LISTING_BODY_BYTES {
        return Err(ModelSyncSkipped {
            reason: format!("{url} returned more than {MAX_LISTING_BODY_BYTES} bytes"),
        });
    }
    serde_json::from_slice::<ModelListing>(&response.body)
        .map(|listing| listing.data)
        .map_err(|_invalid| ModelSyncSkipped {
            reason: format!("{url} returned an unexpected body"),
        })
}

fn opencode_model_entry(model: &ListedModel) -> OpenCodeModel {
    let limit = model
        .max_input_tokens
        .zip(model.max_output_tokens)
        .map(|(context, output)| OpenCodeLimit { context, output });
    OpenCodeModel {
        name: model.id.clone(),
        limit,
    }
}

pub fn opencode_provider_config(
    base_url: &str,
    models: &[ListedModel],
) -> Result<String, ModelSyncSkipped> {
    let mut entries = BTreeMap::new();
    for model in chat_models(models) {
        entries.insert(model.id.clone(), opencode_model_entry(&model));
    }
    let provider = OpenCodeProvider {
        npm: OPENCODE_PROVIDER_NPM,
        name: OPENCODE_PROVIDER_NAME,
        options: OpenCodeProviderOptions {
            base_url: format!("{}/v1", base_url.trim_end_matches('/')),
            api_key: format!("{{env:{OPENAI_API_KEY_ENV}}}"),
        },
        models: entries,
    };
    let config = OpenCodeConfig {
        provider: BTreeMap::from([(OPENCODE_PROVIDER_ID.to_owned(), provider)]),
    };
    serde_json::to_string(&config).map_err(|_invalid| ModelSyncSkipped {
        reason: "could not encode the OpenCode config".to_owned(),
    })
}

#[must_use]
pub fn opencode_model_sync_env(
    base_env: &EnvVars,
    base_url: &str,
    api_key: &SecretString,
    transport: &dyn ModelListingTransport,
) -> ModelSyncResult {
    if base_env.contains_key(OPENCODE_CONFIG_CONTENT_ENV) {
        return ModelSyncResult::Skipped(ModelSyncSkipped {
            reason: format!("{OPENCODE_CONFIG_CONTENT_ENV} is already set"),
        });
    }
    let models = match fetch_model_listing(base_url, api_key, transport) {
        Ok(models) => models,
        Err(skipped) => return ModelSyncResult::Skipped(skipped),
    };
    match opencode_provider_config(base_url, &models) {
        Ok(config) => ModelSyncResult::Env(BTreeMap::from([(
            OPENCODE_CONFIG_CONTENT_ENV.to_owned(),
            config,
        )])),
        Err(skipped) => ModelSyncResult::Skipped(skipped),
    }
}

mod tests;

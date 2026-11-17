use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::ollama_model_metadata::{DEFAULT_API_BASE, LiteLlmGlobals, OllamaSettings};
use secrecy::{ExposeSecret as _, SecretString};

const TAGS_PATH: &str = "/api/tags";
const MODELS_KEY: &str = "models";
const NAME_KEY: &str = "name";
const MODEL_KEY: &str = "model";
const OLLAMA_PREFIX: &str = "ollama/";
const AUTHORIZATION: &str = "Authorization";
const SERVER_SUFFIXES: [&str; 6] = [
    "/api/generate",
    "/api/chat",
    "/api/embed",
    "/api/embeddings",
    "/api/show",
    "/api/tags",
];

// httpx.get's default timeout, which OllamaModelInfo.get_models relies on by passing none
pub const TAGS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagsRequest {
    pub url: String,
    pub headers: Vec<(&'static str, SecretString)>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagsResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait TagsTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn get_tags(&self, request: &TagsRequest) -> Result<TagsResponse, Self::Error>;
}

// OllamaModelInfo.get_models: the names the tags endpoint lists, or the catalog list when it cannot answer
pub struct OllamaModels<'context, Transport> {
    pub settings: &'context OllamaSettings,
    pub globals: &'context LiteLlmGlobals,
    // what litellm.models_by_provider["ollama"] holds
    pub catalog: &'context [String],
    pub transport: &'context Transport,
}

impl<Transport: TagsTransport> OllamaModels<'_, Transport> {
    #[must_use]
    pub fn get_models(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> Vec<String> {
        let request = self.tags_request(api_base, api_key);
        let listed = match self.transport.get_tags(&request) {
            Ok(response) if (200..300).contains(&response.status) => decode_tags(&response.body),
            Ok(_) | Err(_) => None,
        };
        match listed {
            Some(names) => names.into_iter().collect(),
            None => {
                tracing::warn!(
                    "Error retrieving ollama tag endpoint, falling back to the catalog's ollama models"
                );
                self.catalog
                    .iter()
                    .map(|model| format!("{OLLAMA_PREFIX}{model}"))
                    .collect()
            }
        }
    }

    // get_server_api_base: the passed base, OLLAMA_API_BASE, the local server, without a known endpoint suffix
    fn server_api_base(&self, api_base: Option<&str>) -> String {
        let configured = [
            api_base.unwrap_or_default(),
            self.settings.api_base.as_deref().unwrap_or_default(),
        ];
        let base = configured
            .into_iter()
            .find(|base| !base.is_empty())
            .unwrap_or(DEFAULT_API_BASE);
        let trimmed = base.trim_end_matches('/');
        for suffix in SERVER_SUFFIXES {
            if let Some(stripped) = trimmed.strip_suffix(suffix) {
                return stripped.to_owned();
            }
        }
        trimmed.to_owned()
    }

    // get_api_key: the passed key, OLLAMA_API_KEY, litellm.api_key, litellm.openai_key, then the secret manager
    fn selected_key(&self, explicit: Option<&SecretString>) -> Option<SecretString> {
        let candidates = [
            explicit,
            self.settings.api_key.as_ref(),
            self.globals.api_key.as_ref(),
            self.globals.openai_key.as_ref(),
            self.globals.secret_manager_api_key.as_ref(),
        ];
        candidates
            .into_iter()
            .flatten()
            .find(|key| !key.expose_secret().is_empty())
            .cloned()
    }

    // a base alone drops every key, so only a call without one or with its own key resolves credentials
    fn tags_request(&self, api_base: Option<&str>, api_key: Option<&SecretString>) -> TagsRequest {
        let resolved =
            if api_base.is_none() || api_key.is_some_and(|key| !key.expose_secret().is_empty()) {
                self.selected_key(api_key)
            } else {
                None
            };
        let headers = resolved.map_or_else(Vec::new, |key| {
            vec![(
                AUTHORIZATION,
                SecretString::from(format!("Bearer {}", key.expose_secret())),
            )]
        });
        TagsRequest {
            url: format!("{}{TAGS_PATH}", self.server_api_base(api_base)),
            headers,
            timeout: TAGS_TIMEOUT,
        }
    }
}

// validate_environment is a no-op that returns an empty map
#[must_use]
pub fn validate_environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

// get_base_model returns the model unchanged
#[must_use]
pub fn base_model<'model>(model: &'model str) -> &'model str {
    model
}

// a dict with a models list or a bare list gives the entries; a body that is not JSON falls back
fn decode_tags(body: &[u8]) -> Option<Vec<String>> {
    let data: Json = serde_json::from_slice(body).ok()?;
    let entries: Vec<&Json> = match &data {
        Json::Object(fields) => match fields.get(MODELS_KEY) {
            Some(Json::List(models)) => models.iter().collect(),
            _ => Vec::new(),
        },
        Json::List(models) => models.iter().collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => Vec::new(),
    };
    let mut names = BTreeSet::new();
    for entry in entries {
        let Json::Object(fields) = entry else {
            continue;
        };
        // Python's `entry.get("name") or entry.get("model")`: an empty or falsy name falls through
        let name = fields
            .get(NAME_KEY)
            .filter(|value| value.is_truthy())
            .or_else(|| fields.get(MODEL_KEY));
        if let Some(Json::Text(name)) = name {
            names.insert(prefixed(name));
        }
    }
    Some(names.into_iter().collect())
}

fn prefixed(name: &str) -> String {
    if name.starts_with(OLLAMA_PREFIX) {
        name.to_owned()
    } else {
        format!("{OLLAMA_PREFIX}{name}")
    }
}

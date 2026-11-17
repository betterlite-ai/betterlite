use std::collections::BTreeSet;
use std::error::Error;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::ollama_model_metadata::{
    LiteLlmGlobals, OllamaSettings, selected_key, server_api_base,
};
use secrecy::{ExposeSecret as _, SecretString};

const TAGS_PATH: &str = "/api/tags";
const MODELS_KEY: &str = "models";
const NAME_KEY: &str = "name";
const MODEL_KEY: &str = "model";
const OLLAMA_PREFIX: &str = "ollama/";
const AUTHORIZATION: &str = "Authorization";

// httpx.get's default timeout, which OllamaModelInfo.get_models relies on by passing none
pub const TAGS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
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
        // a base alone drops every key, so only a call without one or with its own key resolves credentials
        let key =
            if api_base.is_none() || api_key.is_some_and(|key| !key.expose_secret().is_empty()) {
                selected_key(self.settings, self.globals, api_key)
            } else {
                None
            };
        let request = TagsRequest {
            url: format!("{}{TAGS_PATH}", server_api_base(self.settings, api_base)),
            headers: key.map_or_else(Vec::new, |token| {
                vec![(
                    AUTHORIZATION,
                    SecretString::from(format!("Bearer {}", token.expose_secret())),
                )]
            }),
            timeout: TAGS_TIMEOUT,
        };
        match self.transport.get_tags(&request) {
            Ok(response) if (200..300).contains(&response.status) => decode_tags(&response.body),
            Ok(_) | Err(_) => None,
        }
        .unwrap_or_else(|| self.catalog_models())
    }

    // a failed or unreadable reply leaves the catalog as the only list
    fn catalog_models(&self) -> Vec<String> {
        tracing::warn!(
            "Error retrieving ollama tag endpoint, falling back to the catalog's ollama models"
        );
        self.catalog
            .iter()
            .map(|model| format!("{OLLAMA_PREFIX}{model}"))
            .collect()
    }
}

// a dict with a models list or a bare list gives the entries; a body that is not JSON falls back
#[must_use]
pub fn decode_tags(body: &[u8]) -> Option<Vec<String>> {
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
        // Python's `entry.get("name") or entry.get("model")`: an empty name falls through to the model
        let name = match (fields.get(NAME_KEY), fields.get(MODEL_KEY)) {
            (Some(Json::Text(name)), _) if !name.is_empty() => Some(name.as_str()),
            (_, Some(Json::Text(name))) => Some(name.as_str()),
            _ => None,
        };
        if let Some(found) = name {
            names.insert(prefixed(found));
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

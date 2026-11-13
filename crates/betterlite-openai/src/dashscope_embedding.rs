use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::openai_base_config::Headers;

pub mod error;
pub mod identity;
pub mod reply;
mod tests;

use identity::{FamilyIdentity, FamilyKeyError, resolve_key};

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["dimensions", "encoding_format", "user"];
pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub const AUTHORIZATION_HEADER: &str = "Authorization";
pub const JSON_CONTENT_TYPE: &str = "application/json";
pub const DASHSCOPE_DEFAULT_BASE: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
pub const DASHSCOPE_MISSING_KEY_MESSAGE: &str =
    "DashScope API key is required. Set 'DASHSCOPE_API_KEY' env var or pass api_key explicitly.";

pub struct DashscopeEmbeddingConfig {
    pub identity: FamilyIdentity,
}

impl DashscopeEmbeddingConfig {
    pub fn validate_environment(
        &self,
        api_key: Option<&str>,
        headers: &Headers,
    ) -> Result<Headers, FamilyKeyError> {
        let key = resolve_key(&self.identity, api_key)?;
        let mut resolved = Headers::new();
        resolved.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        resolved.insert(AUTHORIZATION_HEADER.to_owned(), format!("Bearer {key}"));
        resolved.extend(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        Ok(resolved)
    }

    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>) -> String {
        let base = api_base
            .or(self.identity.env_base.as_deref())
            .unwrap_or(&self.identity.default_base);
        let trimmed = base.trim_end_matches('/');
        if trimmed.ends_with("/embeddings") {
            return trimmed.to_owned();
        }
        format!("{trimmed}/embeddings")
    }
}

#[must_use]
pub fn map_openai_params(
    non_default_params: &IndexMap<String, Json>,
    mut optional_params: IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    for (name, value) in non_default_params {
        if *value == Json::Null {
            continue;
        }
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    optional_params
}

#[must_use]
pub fn transform_embedding_request(
    model: &str,
    input: &Json,
    optional_params: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut body = IndexMap::new();
    body.insert("model".to_owned(), Json::Text(model.to_owned()));
    body.insert("input".to_owned(), input.clone());
    for name in SUPPORTED_OPENAI_PARAMS {
        let Some(value) = optional_params.get(name) else {
            continue;
        };
        if *value == Json::Null {
            continue;
        }
        body.insert(name.to_owned(), value.clone());
    }
    body
}

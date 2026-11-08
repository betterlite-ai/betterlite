use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::transport_error::TransportError;
use lru::LruCache;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::huggingface_chat::error::{HuggingfaceChatError, HuggingfaceError};
use crate::openai_base_config::Headers;

const HUB_URL: &str = "https://huggingface.co";
const MAPPING_FIELD: &str = "inferenceProviderMapping";
const EXPAND: &str = "expand";
const MAX_MAPPING_BYTES: usize = 1024 * 1024;
const MAX_CACHED_MAPPINGS: usize = 128;

pub type ProviderMapping = BTreeMap<String, ProviderMappingEntry>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProviderMappingEntry {
    pub status: String,
    #[serde(rename = "providerId")]
    pub provider_id: String,
}

pub trait ProviderMappingSource {
    fn mapping(&self, model_id: &str) -> Result<ProviderMapping, HuggingfaceChatError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubResponse {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

pub trait HubGet {
    fn get(
        &self,
        url: &str,
        headers: &Headers,
        query: &[(&str, &str)],
    ) -> Result<HubResponse, TransportError>;
}

#[derive(Deserialize)]
struct HubModel {
    #[serde(default, rename = "inferenceProviderMapping")]
    inference_provider_mapping: Option<ProviderMapping>,
}

pub struct ProviderMappingCache<G> {
    token: Option<SecretString>,
    hub: G,
    entries: Mutex<LruCache<String, ProviderMapping>>,
}

impl<G: HubGet> ProviderMappingCache<G> {
    #[must_use]
    pub fn new(token: Option<SecretString>, hub: G) -> Self {
        let capacity = NonZeroUsize::new(MAX_CACHED_MAPPINGS).unwrap_or(NonZeroUsize::MIN);
        Self {
            token,
            hub,
            entries: Mutex::new(LruCache::new(capacity)),
        }
    }

    pub fn fetch_provider_mapping(
        &self,
        model: &str,
    ) -> Result<ProviderMapping, HuggingfaceChatError> {
        let url = format!("{HUB_URL}/api/models/{model}");
        let response = self
            .hub
            .get(&url, &self.request_headers(), &[(EXPAND, MAPPING_FIELD)])
            .map_err(|source| HuggingfaceError {
                status_code: 500,
                message: format!("Failed to fetch provider mapping: {source}"),
                headers: Headers::new(),
            })?;
        if response.status_code >= 400 {
            let error = HuggingfaceError {
                status_code: response.status_code,
                message: format!(
                    "Failed to fetch provider mapping: the Hub returned HTTP {}",
                    response.status_code
                ),
                headers: response.headers,
            };
            return Err(HuggingfaceChatError::Huggingface(error));
        }
        decode_provider_mapping(&response.body).map_err(|error| match error {
            HuggingfaceChatError::MappingMissing => HuggingfaceChatError::NoProviderMapping {
                model: model.to_owned(),
            },
            other @ (HuggingfaceChatError::Huggingface(_)
            | HuggingfaceChatError::MissingProviderPrefix { .. }
            | HuggingfaceChatError::NoProviderMapping { .. }
            | HuggingfaceChatError::MappingTooLarge { .. }
            | HuggingfaceChatError::MappingDecode { .. }
            | HuggingfaceChatError::Content(_)
            | HuggingfaceChatError::MessageEncode { .. }) => other,
        })
    }

    fn request_headers(&self) -> Headers {
        let mut headers = Headers::from([("Accept".to_owned(), "application/json".to_owned())]);
        let supplied = self
            .token
            .as_ref()
            .filter(|token| !token.expose_secret().is_empty());
        if let Some(token) = supplied {
            headers.insert(
                "Authorization".to_owned(),
                format!("Bearer {}", token.expose_secret()),
            );
        }
        headers
    }
}

impl<G: HubGet> ProviderMappingSource for ProviderMappingCache<G> {
    fn mapping(&self, model_id: &str) -> Result<ProviderMapping, HuggingfaceChatError> {
        let cached = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(model_id)
            .cloned();
        if let Some(mapping) = cached {
            return Ok(mapping);
        }
        let mapping = self.fetch_provider_mapping(model_id)?;
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .put(model_id.to_owned(), mapping.clone());
        Ok(mapping)
    }
}

pub fn decode_provider_mapping(bytes: &[u8]) -> Result<ProviderMapping, HuggingfaceChatError> {
    if bytes.len() > MAX_MAPPING_BYTES {
        return Err(HuggingfaceChatError::MappingTooLarge {
            limit: MAX_MAPPING_BYTES,
        });
    }
    let model: HubModel = serde_json::from_slice(bytes)
        .map_err(|source| HuggingfaceChatError::MappingDecode { source })?;
    model
        .inference_provider_mapping
        .ok_or(HuggingfaceChatError::MappingMissing)
}

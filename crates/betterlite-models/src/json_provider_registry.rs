use std::{collections::BTreeMap, sync::OnceLock};

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::identity::provider_id::ProviderId;

const ASSET: &[u8] = include_bytes!("json_providers.json");
const MAX_BYTES: usize = 256 * 1024;
const MAX_ENTRIES: usize = 128;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProviderDescriptor {
    pub provider: ProviderId,
    pub base_url: String,
    pub api_key_env: String,
    #[serde(default)]
    pub api_base_env: Option<String>,
    #[serde(default = "default_base_class")]
    pub base_class: String,
    #[serde(default)]
    pub param_mappings: BTreeMap<String, String>,
    #[serde(default)]
    pub constraints: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub special_handling: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub supported_endpoints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JSONProviderProjection<'provider> {
    pub provider: ProviderId,
    pub base_url: &'provider str,
    pub api_key_env: &'provider str,
    pub api_base_env: Option<&'provider str>,
}

impl ProviderDescriptor {
    #[must_use]
    pub fn projection(&self) -> JSONProviderProjection<'_> {
        JSONProviderProjection {
            provider: self.provider,
            base_url: &self.base_url,
            api_key_env: &self.api_key_env,
            api_base_env: self.api_base_env.as_deref(),
        }
    }
}

fn default_base_class() -> String {
    "openai_gpt".to_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    pub bytes: usize,
    pub entries: usize,
    pub depth: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            bytes: MAX_BYTES,
            entries: MAX_ENTRIES,
            depth: MAX_DEPTH,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("JSON provider registry exceeds {limit} bytes")]
    TooLarge { limit: usize },
    #[error("JSON provider registry exceeds {limit} entries")]
    TooManyEntries { limit: usize },
    #[error("JSON provider registry nesting exceeds {limit}")]
    TooDeep { limit: usize },
    #[error("JSON provider registry JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("provider slug {slug} is not registered")]
    UnknownProvider { slug: String },
}

#[derive(Clone, Debug, Default)]
pub struct JSONProviderRegistry {
    pub providers: IndexMap<ProviderId, ProviderDescriptor>,
    loaded: bool,
}

impl JSONProviderRegistry {
    #[must_use]
    pub fn load() -> Self {
        static REGISTRY: OnceLock<JSONProviderRegistry> = OnceLock::new();
        REGISTRY
            .get_or_init(|| {
                let mut registry = Self::default();
                registry.load_from(|| Ok(ASSET.to_vec()));
                registry
            })
            .clone()
    }

    pub fn load_from<Reader>(&mut self, read_asset: Reader)
    where
        Reader: FnOnce() -> std::io::Result<Vec<u8>>,
    {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let bytes = match read_asset() {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, "failed to read JSON provider registry asset");
                return;
            }
        };
        let providers = match decode_json_provider_registry(&bytes, DecodeLimits::default()) {
            Ok(providers) => providers,
            Err(error) => {
                tracing::warn!(%error, "failed to decode JSON provider registry asset");
                return;
            }
        };
        for (slug, raw) in providers {
            let Ok(provider) = slug.parse::<ProviderId>() else {
                tracing::warn!(slug, "unknown provider slug in JSON provider registry");
                break;
            };
            let fields = match serde_json::from_str::<DescriptorFields>(raw.get()) {
                Ok(fields) => fields,
                Err(error) => {
                    tracing::warn!(slug, %error, "malformed JSON provider descriptor");
                    break;
                }
            };
            self.providers.insert(
                provider,
                ProviderDescriptor {
                    provider,
                    base_url: fields.base_url,
                    api_key_env: fields.api_key_env,
                    api_base_env: fields.api_base_env,
                    base_class: fields.base_class,
                    param_mappings: fields.param_mappings,
                    constraints: fields.constraints,
                    special_handling: fields.special_handling,
                    supported_endpoints: fields.supported_endpoints,
                },
            );
        }
    }

    #[must_use]
    pub fn get(&self, provider: ProviderId) -> Option<&ProviderDescriptor> {
        if !self.loaded {
            return None;
        }
        self.providers.get(&provider)
    }

    #[must_use]
    pub fn exists(&self, provider: ProviderId) -> bool {
        match self.get(provider) {
            Some(_) => true,
            None => false,
        }
    }

    #[must_use]
    pub fn get_by_base_url(&self, base_url: &str) -> Option<&ProviderDescriptor> {
        self.providers
            .values()
            .find(|provider| provider.base_url == base_url)
    }

    #[must_use]
    pub fn supports_responses_api(&self, provider: ProviderId) -> bool {
        self.get(provider).is_some_and(|value| {
            value
                .supported_endpoints
                .iter()
                .any(|endpoint| endpoint == "/v1/responses")
        })
    }
}

#[derive(Deserialize)]
struct DescriptorFields {
    base_url: String,
    api_key_env: String,
    #[serde(default)]
    api_base_env: Option<String>,
    #[serde(default = "default_base_class")]
    base_class: String,
    #[serde(default)]
    param_mappings: BTreeMap<String, String>,
    #[serde(default)]
    constraints: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    special_handling: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    supported_endpoints: Vec<String>,
}

pub fn decode_json_provider_registry(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<IndexMap<String, Box<RawValue>>, RegistryError> {
    if bytes.len() > limits.bytes {
        return Err(RegistryError::TooLarge {
            limit: limits.bytes,
        });
    }
    let providers: IndexMap<String, Box<RawValue>> = serde_json::from_slice(bytes)?;
    if providers.len() > limits.entries {
        return Err(RegistryError::TooManyEntries {
            limit: limits.entries,
        });
    }
    for value in providers.values() {
        check_depth(value.get(), 1, limits.depth)?;
    }
    Ok(providers)
}

fn check_depth(value: &str, depth: usize, limit: usize) -> Result<(), RegistryError> {
    let first = value.as_bytes().first().copied();
    if matches!(first, Some(b'{' | b'[')) && depth >= limit {
        return Err(RegistryError::TooDeep { limit });
    }
    if first == Some(b'{') {
        let fields: IndexMap<String, Box<RawValue>> = serde_json::from_str(value)?;
        for child in fields.values() {
            check_depth(child.get(), depth.saturating_add(1), limit)?;
        }
    } else if first == Some(b'[') {
        let elements: Vec<Box<RawValue>> = serde_json::from_str(value)?;
        for child in elements {
            check_depth(child.get(), depth.saturating_add(1), limit)?;
        }
    }
    Ok(())
}

mod tests;

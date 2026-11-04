use std::{collections::BTreeMap, io};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_with::{DefaultOnNull, serde_as};

use crate::identity::provider_id::ProviderId;

const ASSET: &[u8] = include_bytes!("json_providers.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderDescriptor {
    #[serde(rename = "slug")]
    pub provider: ProviderId,
    #[serde(flatten)]
    pub config: ProviderConfig,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key_env: String,
    #[serde(default)]
    pub api_base_env: Option<String>,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull<_>")]
    pub base_class: BaseClass,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull<_>")]
    pub param_mappings: BTreeMap<String, String>,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull<_>")]
    pub constraints: Constraints,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull<_>")]
    pub special_handling: BTreeMap<String, bool>,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull<_>")]
    pub supported_endpoints: Vec<String>,
}

#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Constraints {
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub temperature_max: Option<rust_decimal::Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub temperature_min: Option<rust_decimal::Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub temperature_min_with_n_gt_1: Option<rust_decimal::Decimal>,
    pub cache_control_ttl: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct BaseClass(pub String);

impl Default for BaseClass {
    fn default() -> Self {
        Self("openai_gpt".to_owned())
    }
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
            bytes: 256 * 1024,
            entries: 128,
            depth: 32,
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
    UnknownProvider {
        slug: String,
        #[source]
        source: strum::ParseError,
    },
}

#[derive(Clone, Debug, Default)]
pub struct JSONProviderRegistry {
    pub providers: IndexMap<ProviderId, ProviderDescriptor>,
    pub loaded: bool,
}

impl JSONProviderRegistry {
    #[must_use]
    pub fn load() -> Self {
        let mut registry = Self::default();
        registry.load_from(|| Ok(ASSET.to_vec()));
        registry
    }

    pub fn load_from<Reader>(&mut self, read_asset: Reader)
    where
        Reader: FnOnce() -> io::Result<Vec<u8>>,
    {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let bytes = match read_asset() {
            Ok(bytes) => bytes,
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(%error, "failed to read JSON provider registry asset");
                }
                return;
            }
        };
        match decode_json_provider_registry(&bytes, DecodeLimits::default()) {
            Ok(registry) => self.providers = registry.providers,
            Err(error) => tracing::warn!(%error, "failed to decode JSON provider registry asset"),
        }
    }

    #[must_use]
    pub fn get(&self, slug: &str) -> Option<&ProviderDescriptor> {
        slug.parse::<ProviderId>()
            .map_or(None, |provider| self.providers.get(&provider))
    }

    #[must_use]
    pub fn exists(&self, slug: &str) -> bool {
        let Ok(provider) = slug.parse::<ProviderId>() else {
            return false;
        };
        self.providers.contains_key(&provider)
    }

    #[must_use]
    pub fn get_by_base_url(&self, base_url: &str) -> Option<&ProviderDescriptor> {
        self.providers
            .values()
            .find(|entry| entry.config.base_url == base_url)
    }

    #[must_use]
    pub fn supports_responses_api(&self, slug: &str) -> bool {
        self.get(slug).is_some_and(|entry| {
            entry
                .config
                .supported_endpoints
                .iter()
                .any(|endpoint| endpoint == "/v1/responses")
        })
    }
}

pub fn decode_json_provider_registry(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<JSONProviderRegistry, RegistryError> {
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
        check_depth(value.get(), 1, limits.depth.min(128))?;
    }
    let mut registry = JSONProviderRegistry {
        providers: IndexMap::new(),
        loaded: true,
    };
    for (slug, raw) in providers {
        match descriptor(&slug, raw.get()) {
            Ok(entry) => {
                registry.providers.insert(entry.provider, entry);
            }
            Err(error) => {
                tracing::warn!(%error, "malformed JSON provider descriptor");
                break;
            }
        }
    }
    Ok(registry)
}

fn descriptor(slug: &str, raw: &str) -> Result<ProviderDescriptor, RegistryError> {
    let provider = slug
        .parse()
        .map_err(|source| RegistryError::UnknownProvider {
            slug: slug.to_owned(),
            source,
        })?;
    let config = serde_json::from_str(raw)?;
    Ok(ProviderDescriptor { provider, config })
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
    }
    if first == Some(b'[') {
        let elements: Vec<Box<RawValue>> = serde_json::from_str(value)?;
        for child in elements {
            check_depth(child.get(), depth.saturating_add(1), limit)?;
        }
    }
    Ok(())
}

mod tests;

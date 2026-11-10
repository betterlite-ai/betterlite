use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub mod projection;

mod decoder;
mod tests;

pub const MAX_REGISTRY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_REGISTRY_DEPTH: usize = 8;
pub const MAX_REGISTRY_PROVIDERS: usize = 10_000;
pub const MAX_REGISTRY_ENDPOINTS: usize = 1_024;
pub const ROOT_ASSET: &[u8] = include_bytes!("../fixtures/provider_endpoints_support.json");
pub const PACKAGED_ASSET: &[u8] =
    include_bytes!("../fixtures/provider_endpoints_support_backup.json");

// The shipped registry is the packaged backup; the root asset exists for pinned parity checks.
pub const RUNTIME_ASSET: RegistryAsset = RegistryAsset::Packaged;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryAsset {
    Root,
    Packaged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum EndpointSupport {
    Supported,
    Unsupported,
    Unlisted,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilities {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub endpoints: IndexMap<String, bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EndpointLabel {
    pub docs_label: String,
    pub display_name: String,
    pub leftnav_label: String,
    pub provider_json_field: String,
    pub url: String,
    #[serde(default)]
    pub bridges_to_chat_completion: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RegistrySchema {
    pub provider_slug: SchemaProvider,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SchemaProvider {
    pub display_name: String,
    pub url: String,
    #[serde(default)]
    pub endpoints: IndexMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryDocument {
    #[serde(rename = "_comment", default)]
    comment: Option<String>,
    #[serde(rename = "_schema", default)]
    schema: Option<RegistrySchema>,
    #[serde(default)]
    providers: IndexMap<String, ProviderCapabilities>,
    #[serde(default)]
    endpoints: IndexMap<String, EndpointLabel>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointRegistry {
    pub asset: RegistryAsset,
    pub comment: Option<String>,
    pub schema: Option<RegistrySchema>,
    pub providers: IndexMap<String, ProviderCapabilities>,
    pub endpoints: IndexMap<String, EndpointLabel>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("provider capability registry is {size} bytes, over the {limit} byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("provider capability registry has too many providers: {count} over the {limit} limit")]
    TooManyProviders { count: usize, limit: usize },
    #[error(
        "provider capability registry entry {provider} has too many endpoints: {count} over the {limit} limit"
    )]
    TooManyProviderEndpoints {
        provider: String,
        count: usize,
        limit: usize,
    },
    #[error(
        "provider capability registry has too many endpoint labels: {count} over the {limit} limit"
    )]
    TooManyEndpointLabels { count: usize, limit: usize },
    #[error("provider capability registry nesting exceeds {limit}")]
    TooDeep { limit: usize },
    #[error("provider capability registry JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn parse(bytes: &[u8], asset: RegistryAsset) -> Result<EndpointRegistry, RegistryError> {
    if bytes.len() > MAX_REGISTRY_BYTES {
        return Err(RegistryError::TooLarge {
            size: bytes.len(),
            limit: MAX_REGISTRY_BYTES,
        });
    }
    decoder::validate(bytes)?;
    let document: RegistryDocument = serde_json::from_slice(bytes)?;
    if document.providers.len() > MAX_REGISTRY_PROVIDERS {
        return Err(RegistryError::TooManyProviders {
            count: document.providers.len(),
            limit: MAX_REGISTRY_PROVIDERS,
        });
    }
    for (provider, capabilities) in &document.providers {
        if capabilities.endpoints.len() > MAX_REGISTRY_ENDPOINTS {
            return Err(RegistryError::TooManyProviderEndpoints {
                provider: provider.clone(),
                count: capabilities.endpoints.len(),
                limit: MAX_REGISTRY_ENDPOINTS,
            });
        }
    }
    if document.endpoints.len() > MAX_REGISTRY_ENDPOINTS {
        return Err(RegistryError::TooManyEndpointLabels {
            count: document.endpoints.len(),
            limit: MAX_REGISTRY_ENDPOINTS,
        });
    }
    Ok(EndpointRegistry {
        asset,
        comment: document.comment,
        schema: document.schema,
        providers: document.providers,
        endpoints: document.endpoints,
    })
}

#[must_use]
pub fn support(registry: &EndpointRegistry, provider: &str, endpoint: &str) -> EndpointSupport {
    match registry
        .providers
        .get(provider)
        .and_then(|capabilities| capabilities.endpoints.get(endpoint))
    {
        Some(true) => EndpointSupport::Supported,
        Some(false) => EndpointSupport::Unsupported,
        None => EndpointSupport::Unlisted,
    }
}

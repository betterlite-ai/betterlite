use betterlite_models::decisions::Extra;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::RouterPresetsError;

pub const BUNDLED_CATALOG: &str = include_str!("autorouter_presets.json");
// LiteLLM reads whatever the remote sends; a catalog this large is not a preset list any dashboard can render
pub const MAX_CATALOG_BYTES: usize = 1024 * 1024;

pub type PresetCatalog = IndexMap<String, AutoRouterPresetRecord>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutoRouterPresetTiers {
    #[serde(rename = "SIMPLE")]
    pub simple: Vec<String>,
    #[serde(rename = "MEDIUM")]
    pub medium: Vec<String>,
    #[serde(rename = "COMPLEX")]
    pub complex: Vec<String>,
    #[serde(rename = "REASONING")]
    pub reasoning: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AutoRouterPresetConfig {
    pub tiers: AutoRouterPresetTiers,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AutoRouterPresetRecord {
    pub label: String,
    pub description: String,
    pub complexity_router_config: AutoRouterPresetConfig,
    #[serde(flatten)]
    pub extra: Extra,
}

pub fn decode_catalog(bytes: &[u8]) -> Result<PresetCatalog, RouterPresetsError> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(RouterPresetsError::Oversize {
            size: bytes.len(),
            limit: MAX_CATALOG_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|error| RouterPresetsError::Decode(Box::new(error)))
}

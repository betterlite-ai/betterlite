use std::ptr;

use crate::capabilities::CapabilityFields;
use crate::entry::ModelFields;
use indexmap::IndexMap;

mod tests;

pub const MODEL_COST_KEY_PREFIX: &str = "fireworks_ai/";
const FIREROUTER: &str = "firerouter";
const FIREROUTER_FAMILY: &str = "accounts/fireworks/routers/firerouter";
const FOUNDRY_MODEL_ID_PREFIX: &str = "FW-";
const MODELS_RESOURCE: &str = "accounts/fireworks/models/";
const ROUTERS_RESOURCE: &str = "accounts/fireworks/routers/";
const ROUTER_SHORT_NAMES: [&str; 3] = [FIREROUTER, "auto", "auto-instant"];

pub type Catalog = IndexMap<String, ModelFields>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
pub enum Capability {
    #[serde(rename = "supports_function_calling")]
    FunctionCalling,
    #[serde(rename = "supports_reasoning")]
    Reasoning,
    #[serde(rename = "supports_vision")]
    Vision,
    #[serde(rename = "supports_pdf_input")]
    PdfInput,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct ProviderSpecificModelInfo {
    pub supports_function_calling: bool,
    pub supports_prompt_caching: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_reasoning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_pdf_input: Option<bool>,
}

#[derive(Debug, Default)]
pub struct FireworksMetadata {
    cached: Option<CachedIndex>,
}

#[derive(Debug)]
struct CachedIndex {
    identity: usize,
    generation: u64,
    entries: Vec<IndexEntry>,
}

#[derive(Debug)]
struct IndexEntry {
    short_name: String,
    capabilities: CapabilityFields,
}

#[must_use]
pub fn resolve_resource_name(model: &str) -> String {
    let stripped = model.strip_prefix(MODEL_COST_KEY_PREFIX).unwrap_or(model);
    if stripped.starts_with("accounts/")
        || stripped.starts_with(FOUNDRY_MODEL_ID_PREFIX)
        || stripped.contains('#')
    {
        return stripped.to_owned();
    }
    if stripped.starts_with("routers/") || stripped.starts_with("models/") {
        return format!("accounts/fireworks/{stripped}");
    }
    let firerouter_prefix = format!("{FIREROUTER}/");
    if stripped.ends_with("-fast")
        || ROUTER_SHORT_NAMES.contains(&stripped)
        || stripped.starts_with(&firerouter_prefix)
    {
        return format!("{ROUTERS_RESOURCE}{stripped}");
    }
    format!("{MODELS_RESOURCE}{stripped}")
}

#[must_use]
pub fn capability_exact(catalog: &Catalog, model: &str, capability: Capability) -> Option<bool> {
    let short_name = short_model_name(model);
    let mut candidates = vec![
        model.to_owned(),
        format!("{MODEL_COST_KEY_PREFIX}{short_name}"),
        format!("{MODEL_COST_KEY_PREFIX}{MODELS_RESOURCE}{short_name}"),
    ];
    if let Some(family_key) = firerouter_family_cost_key(model) {
        candidates.push(family_key);
    }
    candidates.iter().find_map(|candidate| {
        let fields = catalog.get(candidate)?;
        capability_value(&fields.capabilities, capability)
    })
}

impl FireworksMetadata {
    #[must_use]
    pub fn get_model_cost_capability(
        &mut self,
        catalog: &Catalog,
        generation: u64,
        model: &str,
        capability: Capability,
    ) -> Option<bool> {
        if let Some(exact) = capability_exact(catalog, model, capability) {
            return Some(exact);
        }
        let short_name = short_model_name(model);
        let index = self.index(catalog, generation);
        let mut best: Option<(usize, bool)> = None;
        for entry in index {
            let Some(value) = capability_value(&entry.capabilities, capability) else {
                continue;
            };
            if !matches_on_hyphen_boundary(&short_name, &entry.short_name) {
                continue;
            }
            let length = entry.short_name.chars().count();
            if best
                .as_ref()
                .is_none_or(|(best_length, _)| length > *best_length)
            {
                best = Some((length, value));
            }
        }
        best.map(|(_, value)| value)
    }

    #[must_use]
    pub fn get_provider_info(
        &mut self,
        catalog: &Catalog,
        generation: u64,
        model: &str,
    ) -> ProviderSpecificModelInfo {
        let function_calling =
            self.get_model_cost_capability(catalog, generation, model, Capability::FunctionCalling);
        let reasoning =
            self.get_model_cost_capability(catalog, generation, model, Capability::Reasoning);
        let vision = self.get_model_cost_capability(catalog, generation, model, Capability::Vision);
        let pdf_input =
            self.get_model_cost_capability(catalog, generation, model, Capability::PdfInput);
        ProviderSpecificModelInfo {
            supports_function_calling: function_calling.unwrap_or(true),
            supports_prompt_caching: true,
            supports_reasoning: reasoning.filter(|value| *value),
            supports_vision: vision,
            supports_pdf_input: pdf_input,
        }
    }

    fn index(&mut self, catalog: &Catalog, generation: u64) -> &[IndexEntry] {
        let identity = ptr::from_ref(catalog).addr();
        let stale = self
            .cached
            .as_ref()
            .is_none_or(|cached| cached.identity != identity || cached.generation != generation);
        if stale {
            self.cached = None;
        }
        let cached = self.cached.get_or_insert_with(|| CachedIndex {
            identity,
            generation,
            entries: build_index(catalog),
        });
        &cached.entries
    }
}

fn short_model_name(model: &str) -> String {
    let stripped = model.strip_prefix(MODEL_COST_KEY_PREFIX).unwrap_or(model);
    stripped
        .strip_prefix(MODELS_RESOURCE)
        .unwrap_or(stripped)
        .to_owned()
}

fn firerouter_family_cost_key(model: &str) -> Option<String> {
    let resolved = resolve_resource_name(model);
    let family_prefix = format!("{FIREROUTER_FAMILY}/");
    resolved
        .starts_with(&family_prefix)
        .then(|| format!("{MODEL_COST_KEY_PREFIX}{FIREROUTER_FAMILY}"))
}

fn matches_on_hyphen_boundary(short_name: &str, key_short: &str) -> bool {
    let leading = format!("{key_short}-");
    let trailing = format!("-{key_short}");
    let middle = format!("-{key_short}-");
    short_name == key_short
        || short_name.starts_with(&leading)
        || short_name.ends_with(&trailing)
        || short_name.contains(&middle)
}

const fn capability_value(capabilities: &CapabilityFields, capability: Capability) -> Option<bool> {
    match capability {
        Capability::FunctionCalling => capabilities.tooling.supports_function_calling,
        Capability::Reasoning => capabilities.reasoning.supports_reasoning,
        Capability::Vision => capabilities.media.supports_vision,
        Capability::PdfInput => capabilities.media.supports_pdf_input,
    }
}

fn build_index(catalog: &Catalog) -> Vec<IndexEntry> {
    catalog
        .iter()
        .filter_map(|(key, fields)| {
            let without_provider = key.strip_prefix(MODEL_COST_KEY_PREFIX)?;
            let short_name = without_provider
                .strip_prefix(MODELS_RESOURCE)
                .unwrap_or(without_provider);
            if short_name.is_empty() {
                return None;
            }
            Some(IndexEntry {
                short_name: short_name.to_owned(),
                capabilities: fields.capabilities.clone(),
            })
        })
        .collect()
}

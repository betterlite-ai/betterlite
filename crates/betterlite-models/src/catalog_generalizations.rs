pub(crate) mod python_text;
mod rule;

use std::collections::{BTreeMap, BTreeSet};

use fancy_regex::Regex;
use serde_json::value::RawValue;

use crate::catalog_generalizations::rule::{CompiledRule, compile_rule, resolve_legacy_extends};
use crate::error::ModelsError;

const MAX_RULES_BYTES: usize = 0x0010_0000;
const PROVIDER_KEY: &str = "litellm_provider";

pub type ModelInfo = BTreeMap<String, Box<RawValue>>;

#[derive(Debug, Default)]
pub struct CatalogGeneralizations {
    pub rules: Vec<Box<RawValue>>,
    pub routing: Vec<RoutingRule>,
    pub capabilities: Vec<CapabilityRule>,
}

#[derive(Debug)]
pub struct RoutingRule {
    pub pattern: Regex,
    pub provider: String,
}

#[derive(Debug)]
pub struct CapabilityRule {
    pub pattern: Regex,
    pub model_info: ModelInfo,
    pub fill_missing_for_providers: BTreeSet<String>,
}

pub fn decode_model_generalization_rules(bytes: &[u8]) -> Result<Vec<Box<RawValue>>, ModelsError> {
    let size = bytes.len();
    if size > MAX_RULES_BYTES {
        return Err(ModelsError::RulesTooLarge { size });
    }
    let document: Box<RawValue> =
        serde_json::from_slice(bytes).map_err(|source| ModelsError::RulesJson { source })?;
    // LiteLLM installs an empty rule list when the catalog's rules value is not a list
    Ok(serde_json::from_str(document.get()).unwrap_or_default())
}

fn searches(pattern: &Regex, model: &str) -> bool {
    match pattern.is_match(model) {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(pattern = pattern.as_str(), %error, "skipping fallback generalization rule whose regex search failed");
            false
        }
    }
}

impl CatalogGeneralizations {
    #[must_use]
    pub fn install(rules: Vec<Box<RawValue>>) -> (Self, Vec<ModelsError>) {
        let mut installed = Self::default();
        let mut warnings = Vec::new();
        for resolved in resolve_legacy_extends(&rules) {
            match compile_rule(resolved) {
                Ok(CompiledRule::Routing(routing)) => installed.routing.push(routing),
                Ok(CompiledRule::Capability(capability)) => installed.capabilities.push(capability),
                Ok(CompiledRule::Both(routing, capability)) => {
                    installed.routing.push(routing);
                    installed.capabilities.push(capability);
                }
                Err(warning) => warnings.push(warning),
            }
        }
        installed.rules = rules;
        (installed, warnings)
    }

    #[must_use]
    pub fn match_routing(&self, model: &str) -> Option<&str> {
        if model.is_empty() {
            return None;
        }
        let routing = &self.routing;
        let first = routing.iter().find(|rule| searches(&rule.pattern, model))?;
        Some(&first.provider)
    }

    #[must_use]
    pub fn match_capabilities(&self, model: &str) -> Option<ModelInfo> {
        if model.is_empty() {
            return None;
        }
        let mut union: Option<ModelInfo> = None;
        for rule in &self.capabilities {
            if searches(&rule.pattern, model) {
                let merged = union.get_or_insert_default();
                merged.extend(rule.model_info.clone());
            }
        }
        union
    }

    #[must_use]
    pub fn match_fill_missing(&self, model: &str, provider: &str) -> Option<ModelInfo> {
        if model.is_empty() || provider.is_empty() {
            return None;
        }
        let mut fill = ModelInfo::new();
        for rule in &self.capabilities {
            let listed = rule.fill_missing_for_providers.contains(provider);
            if listed && searches(&rule.pattern, model) {
                fill.extend(rule.model_info.clone());
            }
        }
        fill.remove(PROVIDER_KEY);
        (!fill.is_empty()).then_some(fill)
    }
}

mod tests;

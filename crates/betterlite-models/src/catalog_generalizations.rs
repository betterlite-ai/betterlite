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
    if bytes.len() > MAX_RULES_BYTES {
        return Err(ModelsError::RulesTooLarge {
            size: bytes.len(),
            limit: MAX_RULES_BYTES,
        });
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
        let mut routing = Vec::new();
        let mut capabilities = Vec::new();
        let mut warnings = Vec::new();
        for resolved in resolve_legacy_extends(&rules) {
            match compile_rule(resolved) {
                Ok(CompiledRule::Routing(routing_rule)) => routing.push(routing_rule),
                Ok(CompiledRule::Capability(capability_rule)) => capabilities.push(capability_rule),
                Ok(CompiledRule::Both(routing_rule, capability_rule)) => {
                    routing.push(routing_rule);
                    capabilities.push(capability_rule);
                }
                Err(warning) => warnings.push(warning),
            }
        }
        (
            Self {
                rules,
                routing,
                capabilities,
            },
            warnings,
        )
    }

    #[must_use]
    pub fn match_routing(&self, model: &str) -> Option<&str> {
        if model.is_empty() {
            return None;
        }
        let first = self
            .routing
            .iter()
            .find(|rule| searches(&rule.pattern, model))?;
        Some(&first.provider)
    }

    #[must_use]
    pub fn match_capabilities(&self, model: &str) -> Option<ModelInfo> {
        if model.is_empty() {
            return None;
        }
        let mut union: Option<ModelInfo> = None;
        for rule in self
            .capabilities
            .iter()
            .filter(|rule| searches(&rule.pattern, model))
        {
            union
                .get_or_insert_default()
                .extend(rule.model_info.clone());
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
            if rule.fill_missing_for_providers.contains(provider) && searches(&rule.pattern, model)
            {
                let fields = rule
                    .model_info
                    .iter()
                    .filter(|(key, _)| key.as_str() != PROVIDER_KEY);
                fill.extend(fields.map(|(key, value)| (key.clone(), value.clone())));
            }
        }
        (!fill.is_empty()).then_some(fill)
    }
}

mod tests;

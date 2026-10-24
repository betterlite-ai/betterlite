use std::collections::{BTreeMap, BTreeSet};

use fancy_regex::RegexBuilder;
use serde_json::value::RawValue;

use crate::catalog_generalizations::python_text::python_text;
use crate::catalog_generalizations::{CapabilityRule, ModelInfo, PROVIDER_KEY, RoutingRule};
use crate::error::ModelsError;

pub(super) struct RuleFields {
    name: Option<Box<RawValue>>,
    pattern: Option<Box<RawValue>>,
    model_info: Option<ModelInfo>,
    extends: Option<String>,
    fill_missing_for_providers: Option<Box<RawValue>>,
}

pub(super) enum CompiledRule {
    Routing(RoutingRule),
    Capability(CapabilityRule),
    Both(RoutingRule, CapabilityRule),
}

impl RuleFields {
    fn parse(raw: &RawValue) -> Option<Self> {
        let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(raw.get()).ok()?;
        let model_info = fields
            .remove("model_info")
            .and_then(|info| serde_json::from_str(info.get()).ok());
        let extends = fields
            .remove("extends")
            .and_then(|parent| serde_json::from_str(parent.get()).ok());
        Some(Self {
            name: fields.remove("name"),
            pattern: fields.remove("pattern"),
            model_info,
            extends,
            fill_missing_for_providers: fields.remove("fill_missing_for_providers"),
        })
    }
}

pub(super) fn resolve_legacy_extends(rules: &[Box<RawValue>]) -> Vec<RuleFields> {
    let parsed: Vec<RuleFields> = rules
        .iter()
        .filter_map(|raw| RuleFields::parse(raw))
        .collect();
    let mut parents = BTreeMap::new();
    for rule in &parsed {
        let name = rule
            .name
            .as_ref()
            .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok());
        if let (Some(parent_name), Some(parent_info)) = (name, &rule.model_info) {
            parents.insert(parent_name, parent_info.clone());
        }
    }
    parsed
        .into_iter()
        .map(|mut rule| {
            if let Some(parent_info) = rule
                .extends
                .as_ref()
                .and_then(|parent_name| parents.get(parent_name))
            {
                rule.model_info = rule.model_info.map(|own_info| {
                    let mut merged = parent_info.clone();
                    merged.extend(own_info);
                    merged
                });
            }
            rule
        })
        .collect()
}

pub(super) fn compile_rule(rule: RuleFields) -> Result<CompiledRule, ModelsError> {
    let label = python_text(rule.name.as_deref().or(rule.pattern.as_deref()));
    let regex_source = rule
        .pattern
        .as_ref()
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok());
    let (Some(source), Some(model_info)) = (regex_source, rule.model_info) else {
        return Err(ModelsError::MalformedRule { label });
    };
    let pattern = RegexBuilder::new(&source)
        .case_insensitive(true)
        .build()
        .map_err(|error| ModelsError::InvalidRegex {
            pattern: source,
            source: Box::new(error),
        })?;
    let fill_missing: Option<Result<BTreeSet<String>, _>> = rule
        .fill_missing_for_providers
        .map(|raw| serde_json::from_str(raw.get()));
    let fill_missing_for_providers = match fill_missing {
        None => BTreeSet::new(),
        Some(Ok(providers)) => providers,
        Some(Err(_)) => return Err(ModelsError::MalformedFillMissing { label }),
    };
    let capability = CapabilityRule {
        pattern,
        model_info,
        fill_missing_for_providers,
    };
    classify(capability, label)
}

fn classify(capability: CapabilityRule, label: String) -> Result<CompiledRule, ModelsError> {
    let Some(provider_value) = capability.model_info.get(PROVIDER_KEY) else {
        return Ok(CompiledRule::Capability(capability));
    };
    let Ok(provider) = serde_json::from_str::<String>(provider_value.get()) else {
        return Err(ModelsError::InvalidProvider { label });
    };
    let routing = RoutingRule {
        pattern: capability.pattern.clone(),
        provider,
    };
    if capability.model_info.len() == 1 {
        return Ok(CompiledRule::Routing(routing));
    }
    Ok(CompiledRule::Both(routing, capability))
}

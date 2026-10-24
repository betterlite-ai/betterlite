use std::collections::{BTreeMap, BTreeSet};

use fancy_regex::RegexBuilder;
use serde_json::value::RawValue;

use crate::catalog_generalizations::python_text::python_text;
use crate::catalog_generalizations::{CapabilityRule, ModelInfo, PROVIDER_KEY, RoutingRule};
use crate::error::ModelsError;

pub(super) struct RuleFields {
    label: String,
    name: Option<String>,
    pattern: Option<String>,
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
        let name = fields.remove("name");
        let pattern = fields.remove("pattern");
        let model_info = fields
            .remove("model_info")
            .map(|info| serde_json::from_str(info.get()));
        let extends = fields
            .remove("extends")
            .map(|parent| serde_json::from_str(parent.get()));
        Some(Self {
            label: python_text(name.as_deref().or(pattern.as_deref())),
            name: name.and_then(|text| serde_json::from_str(text.get()).ok()),
            pattern: pattern.and_then(|text| serde_json::from_str(text.get()).ok()),
            model_info: model_info.and_then(Result::ok),
            extends: extends.and_then(Result::ok),
            fill_missing_for_providers: fields.remove("fill_missing_for_providers"),
        })
    }
}

pub(super) fn resolve_legacy_extends(rules: &[Box<RawValue>]) -> Vec<RuleFields> {
    let mut parsed = Vec::new();
    let mut parents = BTreeMap::new();
    for rule in rules.iter().filter_map(|raw| RuleFields::parse(raw)) {
        if let (Some(name), Some(parent_info)) = (&rule.name, &rule.model_info) {
            parents.insert(name.clone(), parent_info.clone());
        }
        parsed.push(rule);
    }
    for rule in &mut parsed {
        let parent = rule.extends.as_ref().and_then(|name| parents.get(name));
        if let Some(parent_info) = parent
            && let Some(own_info) = rule.model_info.take()
        {
            let mut merged = parent_info.clone();
            merged.extend(own_info);
            rule.model_info = Some(merged);
        }
    }
    parsed
}

pub(super) fn compile_rule(rule: RuleFields) -> Result<CompiledRule, ModelsError> {
    let label = rule.label;
    let (Some(source), Some(model_info)) = (rule.pattern, rule.model_info) else {
        return Err(ModelsError::MalformedRule { label });
    };
    let built = RegexBuilder::new(&source).case_insensitive(true).build();
    let pattern = built.map_err(|error| ModelsError::InvalidRegex {
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

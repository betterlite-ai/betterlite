use std::collections::BTreeMap;

use regex::Regex;

use super::error::ToolPermissionError;
use super::rule::ToolPermissionRule;

mod tests;

// LiteLLM matches with Python's fullmatch, so the pattern must cover the whole value
#[derive(Debug, Clone)]
pub struct CompiledPattern {
    pub source: String,
    pub regex: Regex,
}

impl CompiledPattern {
    pub fn compile(source: &str) -> Result<Self, regex::Error> {
        let regex = Regex::new(&format!(r"\A(?:{source})\z"))?;
        Ok(Self {
            source: source.to_owned(),
            regex,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompiledTargets {
    pub tool_name: Option<CompiledPattern>,
    pub tool_type: Option<CompiledPattern>,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledRules {
    pub targets: BTreeMap<String, CompiledTargets>,
    pub patterns: BTreeMap<String, BTreeMap<String, CompiledPattern>>,
}

impl CompiledRules {
    pub fn compile_rule(&mut self, rule: &ToolPermissionRule) -> Result<(), ToolPermissionError> {
        let mut targets = CompiledTargets::default();
        if let Some(source) = rule.tool_name.as_deref() {
            targets.tool_name = Some(compile_pattern(rule, "tool_name", source)?);
        }
        if let Some(source) = rule.tool_type.as_deref() {
            targets.tool_type = Some(compile_pattern(rule, "tool_type", source)?);
        }
        let mut patterns = BTreeMap::new();
        for (path, source) in rule.allowed_param_patterns.iter().flatten() {
            patterns.insert(
                path.clone(),
                compile_pattern(rule, "allowed_param_patterns", source)?,
            );
        }
        self.targets.insert(rule.id.clone(), targets);
        // LiteLLM only stores a duplicate rule's patterns when it has some, so an empty set keeps the earlier entry
        if !patterns.is_empty() {
            self.patterns.insert(rule.id.clone(), patterns);
        }
        Ok(())
    }
}

fn compile_pattern(
    rule: &ToolPermissionRule,
    field: &'static str,
    source: &str,
) -> Result<CompiledPattern, ToolPermissionError> {
    CompiledPattern::compile(source).map_err(|error| ToolPermissionError::InvalidRegex {
        rule_id: rule.id.clone(),
        field: field.to_owned(),
        pattern: source.to_owned(),
        source: error,
    })
}

#[must_use]
pub fn matches_regex(pattern: Option<&CompiledPattern>, value: Option<&str>) -> bool {
    match pattern {
        None => true,
        Some(pattern) => value.is_some_and(|text| pattern.regex.is_match(text)),
    }
}

#[must_use]
pub fn rule_matches_tool(
    compiled: &CompiledRules,
    rule: &ToolPermissionRule,
    tool_name: Option<&str>,
    tool_type: Option<&str>,
) -> (bool, bool) {
    let targets = compiled.targets.get(&rule.id);
    let name_pattern = targets.and_then(|targets| targets.tool_name.as_ref());
    let type_pattern = targets.and_then(|targets| targets.tool_type.as_ref());
    let name_required = rule.tool_name.is_some();
    let type_required = rule.tool_type.is_some();
    let name_matched = !name_required || matches_regex(name_pattern, tool_name);
    let type_matched = !type_required || matches_regex(type_pattern, tool_type);
    (name_matched && type_matched, name_required && name_matched)
}

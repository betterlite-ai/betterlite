use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::super::matching::CompiledRules;
use super::super::rule::ToolPermissionRule;
use super::paths::collect_argument_paths;

// LiteLLM's _patterns_match_for_rule: every configured path needs a collected value that fullmatches
pub(super) fn patterns_match_for_rule(
    compiled: &CompiledRules,
    rule: &ToolPermissionRule,
    arguments: &BTreeMap<String, GuardrailValue>,
    tool_name: Option<&str>,
) -> Option<String> {
    let patterns = compiled.patterns.get(&rule.id)?;
    if patterns.is_empty() {
        return None;
    }
    let mut collected = BTreeMap::new();
    collect_argument_paths(arguments, &mut collected);
    for (path, pattern) in patterns {
        let Some(values) = collected.get(path) else {
            let rule_id = rule.id.as_str();
            return Some(format!(
                "Missing value for path '{path}' required by rule '{rule_id}'"
            ));
        };
        for value in values {
            if !pattern.regex.is_match(value) {
                let pattern_source = pattern.source.as_str();
                let tool = tool_name.unwrap_or("unknown_tool");
                return Some(format!(
                    "Value '{value}' for path '{path}' does not match allowed pattern \
                     '{pattern_source}' for tool '{tool}'"
                ));
            }
        }
    }
    None
}

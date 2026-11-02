use std::collections::BTreeSet;

use crate::cisco_configuration::records::CiscoVerdict;
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict_enforcement::support::{insert_serialized, rule_name};

// Maps a request's top-level fields onto the shorter names LiteLLM's fn _log_decision adds to the CISCO_AI_DEFENSE_DECISION line, when present and non-empty.
pub(crate) const REQUEST_CONTEXT_KEYS: [(&str, &str); 4] = [
    ("model", "model"),
    ("litellm_call_id", "call_id"),
    ("mcp_tool_name", "mcp_tool"),
    ("mcp_server_name", "mcp_server"),
];

pub(crate) fn rule_violations(rules: &[Fields]) -> Vec<String> {
    let mut violations: BTreeSet<String> = BTreeSet::new();
    for rule in rules {
        let Some(name) = rule_name(rule).filter(|name| !name.is_empty()) else {
            continue;
        };
        let classification = Json::field(rule, "classification");
        let excluded = matches!(classification, Json::Null)
            || matches!(&classification, Json::Text(value) if value == "NONE_VIOLATION");
        if !excluded {
            violations.insert(name);
        }
    }
    violations.into_iter().collect()
}

// Backs LiteLLM's fn _log_decision: each verdict-derived field is dropped when empty, the same way LiteLLM's `v not in (None, [], "")` filter does.
pub(crate) fn insert_verdict_log_fields(fields: &mut Fields, verdict: &CiscoVerdict) {
    if let Some(is_safe) = verdict.is_safe {
        insert_serialized(fields, "is_safe", &is_safe);
    }
    if let Some(severity) = verdict
        .severity
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        insert_serialized(fields, "severity", severity);
    }
    if !verdict.classifications.is_empty() {
        insert_serialized(fields, "classifications", &verdict.classifications);
    }
    let violations = rule_violations(&verdict.rules);
    if !violations.is_empty() {
        insert_serialized(fields, "rule_violations", &violations);
    }
    if let Some(event_id) = verdict
        .event_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        insert_serialized(fields, "event_id", event_id);
    }
}

mod tests;

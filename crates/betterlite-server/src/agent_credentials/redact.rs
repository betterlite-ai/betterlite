use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_redact::structured::SensitiveDataMasker;

use crate::agent_credentials::REDACTED_BY_LITELM;

const MAX_DEPTH: usize = 10;

#[must_use]
// ast-grep-ignore: no-long-names
pub fn redact_sensitive_agent_litellm_params(
    params: Option<&AgentJson>,
    masker: &SensitiveDataMasker,
) -> Option<AgentJson> {
    let value = params?;
    match value {
        AgentJson::String(text) => Some(AgentJson::String(redacted_json_text(text, masker))),
        AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_) => Some(redacted_tree(value, masker, 0)),
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn redact_agent_litellm_params_map(
    params: &BTreeMap<String, AgentJson>,
    masker: &SensitiveDataMasker,
) -> BTreeMap<String, AgentJson> {
    params
        .iter()
        .map(|(key, value)| (key.clone(), redacted_field(key, value, masker, 0)))
        .collect()
}

fn redacted_json_text(text: &str, masker: &SensitiveDataMasker) -> String {
    let Ok(parsed) = serde_json::from_str::<AgentJson>(text) else {
        return String::from(REDACTED_BY_LITELM);
    };
    if !matches!(parsed, AgentJson::Object(_)) {
        return String::from(REDACTED_BY_LITELM);
    }
    redacted_tree(&parsed, masker, 1).to_string()
}

fn redacted_tree(value: &AgentJson, masker: &SensitiveDataMasker, depth: usize) -> AgentJson {
    if depth >= MAX_DEPTH {
        return AgentJson::String(String::from(REDACTED_BY_LITELM));
    }
    match value {
        AgentJson::Array(items) => AgentJson::Array(
            items
                .iter()
                .map(|item| redacted_tree(item, masker, depth.saturating_add(1)))
                .collect(),
        ),
        AgentJson::Object(fields) => AgentJson::Object(
            fields
                .iter()
                .map(|(key, nested)| (key.clone(), redacted_field(key, nested, masker, depth)))
                .collect(),
        ),
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Number(_) | AgentJson::String(_) => {
            value.clone()
        }
    }
}

fn redacted_field(
    key: &str,
    value: &AgentJson,
    masker: &SensitiveDataMasker,
    depth: usize,
) -> AgentJson {
    if masker.is_sensitive_key(key, &[]) {
        return AgentJson::String(String::from(REDACTED_BY_LITELM));
    }
    redacted_tree(value, masker, depth.saturating_add(1))
}

mod tests;

#![expect(
    clippy::disallowed_types,
    reason = "OpenCode configuration overrides are caller-supplied JSON with no fixed shape"
)]
use std::num::FpCategory;

use serde_json::Value;

use crate::errors::HarnessError;
use crate::types::JsonObject;

const MANAGED_CONFIG_KEYS: [&str; 8] = [
    "provider",
    "model",
    "small_model",
    "permission",
    "tools",
    "enabled_providers",
    "disabled_providers",
    "plugin",
];
const AGENT_MANAGED_KEYS: [&str; 3] = ["permission", "tools", "model"];

pub(super) fn validate_user_config(config: &JsonObject) -> Result<(), HarnessError> {
    if let Some(key) = config
        .keys()
        .find(|key| MANAGED_CONFIG_KEYS.contains(&key.as_str()))
    {
        return Err(HarnessError::OpenCodeManagedTopLevelKey { key: key.clone() });
    }
    for section in ["agent", "mode"] {
        validate_agent_section(config, section)?;
    }
    Ok(())
}

fn validate_agent_section(config: &JsonObject, section: &str) -> Result<(), HarnessError> {
    let Some(entries) = config.get(section).filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let Some(agents) = entries.as_object() else {
        return Err(HarnessError::OpenCodeSectionNotMapping {
            section: section.to_owned(),
        });
    };
    for (agent_name, agent) in agents {
        match agent_managed_keys(agent) {
            Ok(managed) if managed.is_empty() => {}
            Ok(managed) => {
                return Err(HarnessError::OpenCodeAgentManagedKeys {
                    section: section.to_owned(),
                    agent: agent_name.clone(),
                    keys: managed,
                });
            }
            Err(AgentShapeCrash) => {
                return Err(HarnessError::OpenCodeAgentShapeInvalid {
                    section: section.to_owned(),
                    agent: agent_name.clone(),
                });
            }
        }
    }
    Ok(())
}

// Mirrors Python's frozenset(agent or ()): a nonzero number, true, or a list holding an object or a list has no Rust set equivalent and crashes LiteLLM instead.
struct AgentShapeCrash;

fn agent_managed_keys(agent: &Value) -> Result<Vec<String>, AgentShapeCrash> {
    if agent.is_null() || agent.as_str().is_some() {
        return Ok(Vec::new());
    }
    if let Some(flag) = agent.as_bool() {
        return if flag {
            Err(AgentShapeCrash)
        } else {
            Ok(Vec::new())
        };
    }
    if let Some(number) = agent.as_f64() {
        let falsy = matches!(number.classify(), FpCategory::Zero);
        return if falsy {
            Ok(Vec::new())
        } else {
            Err(AgentShapeCrash)
        };
    }
    if let Some(items) = agent.as_array() {
        return managed_from_list(items);
    }
    let map = agent.as_object().ok_or(AgentShapeCrash)?;
    Ok(sorted_managed(map.keys().map(String::as_str)))
}

fn managed_from_list(items: &[Value]) -> Result<Vec<String>, AgentShapeCrash> {
    let mut hashable = Vec::with_capacity(items.len());
    for item in items {
        if item.is_object() || item.is_array() {
            return Err(AgentShapeCrash);
        }
        if let Some(text) = item.as_str() {
            hashable.push(text);
        }
    }
    Ok(sorted_managed(hashable.into_iter()))
}

fn sorted_managed<'items, Members>(members: Members) -> Vec<String>
where
    Members: Iterator<Item = &'items str>,
{
    let mut managed: Vec<String> = members
        .filter(|member| AGENT_MANAGED_KEYS.contains(member))
        .map(str::to_owned)
        .collect();
    managed.sort();
    managed
}

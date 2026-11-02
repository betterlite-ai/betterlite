use std::num::FpCategory;

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
        let shape = if agent.is_null() || agent.as_str().is_some() {
            Ok(Vec::new())
        } else if let Some(flag) = agent.as_bool() {
            if flag {
                Err(AgentShapeCrash)
            } else {
                Ok(Vec::new())
            }
        } else if let Some(number) = agent.as_f64() {
            if matches!(number.classify(), FpCategory::Zero) {
                Ok(Vec::new())
            } else {
                Err(AgentShapeCrash)
            }
        } else if let Some(items) = agent.as_array() {
            if items.iter().any(|item| item.is_object() || item.is_array()) {
                Err(AgentShapeCrash)
            } else {
                Ok(sorted_managed(
                    items.iter().filter_map(|item| item.as_str()),
                ))
            }
        } else {
            agent.as_object().map_or(Err(AgentShapeCrash), |map| {
                Ok(sorted_managed(map.keys().map(String::as_str)))
            })
        };
        report_agent_shape(section, agent_name, shape)?;
    }
    Ok(())
}

fn report_agent_shape(
    section: &str,
    agent_name: &str,
    shape: Result<Vec<String>, AgentShapeCrash>,
) -> Result<(), HarnessError> {
    match shape {
        Ok(managed) if managed.is_empty() => Ok(()),
        Ok(keys) => Err(HarnessError::OpenCodeAgentManagedKeys {
            section: section.to_owned(),
            agent: agent_name.to_owned(),
            keys,
        }),
        Err(AgentShapeCrash) => Err(HarnessError::OpenCodeAgentShapeInvalid {
            section: section.to_owned(),
            agent: agent_name.to_owned(),
        }),
    }
}

// Mirrors Python's frozenset(agent or ()): a nonzero number, true, or a list holding an object or a list has no Rust set equivalent and crashes LiteLLM instead.
struct AgentShapeCrash;

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

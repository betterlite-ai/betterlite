use crate::errors::HarnessError;
use crate::types::{Harness, JsonObject};

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
    let managed: Vec<String> = config
        .keys()
        .filter(|key| MANAGED_CONFIG_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    if let Some(key) = managed.first() {
        let reported_key = if managed.len() == 1 {
            key.clone()
        } else {
            managed.join(", ")
        };
        return Err(HarnessError::ManagedConfigKey {
            key: reported_key,
            harness: Harness::OpenCode,
        });
    }
    for section in ["agent", "mode"] {
        validate_agent_section(config, section)?;
    }
    Ok(())
}

fn validate_agent_section(config: &JsonObject, section: &str) -> Result<(), HarnessError> {
    let Some(entries) = config.get(section) else {
        return Ok(());
    };
    let Some(agents) = entries.as_object() else {
        return Err(HarnessError::OptionsMismatch {
            given: "OpenCodeOptions.config[agent/mode]",
            harness: Harness::OpenCode,
            expected: "a mapping of names to agent config",
        });
    };
    for (agent_name, agent) in agents {
        let Some(fields) = agent.as_object() else {
            return Err(HarnessError::OptionsMismatch {
                given: "OpenCodeOptions.config[agent/mode]",
                harness: Harness::OpenCode,
                expected: "a mapping of names to agent config",
            });
        };
        let mut keys: Vec<String> = fields
            .keys()
            .filter(|key| AGENT_MANAGED_KEYS.contains(&key.as_str()))
            .cloned()
            .collect();
        if !keys.is_empty() {
            keys.sort();
            return Err(HarnessError::ManagedConfigKey {
                key: format!("{section}.{agent_name}: {}", keys.join(", ")),
                harness: Harness::OpenCode,
            });
        }
    }
    Ok(())
}

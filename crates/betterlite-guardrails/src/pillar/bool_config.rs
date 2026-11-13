use betterlite_config::loader::EnvironmentLookup;

use super::error::PillarError;

const TRUE_VALUES: [&str; 5] = ["true", "1", "yes", "y", "on"];
const FALSE_VALUES: [&str; 5] = ["false", "0", "no", "n", "off"];

// LiteLLM's _parse_bool_value takes bool, int or str, and so does the resolver's provided value
#[derive(Debug, Clone, PartialEq)]
pub enum BoolSetting {
    Boolean(bool),
    Integer(i64),
    Text(String),
}

pub fn parse_bool_value(value: &BoolSetting) -> Result<bool, PillarError> {
    match value {
        BoolSetting::Boolean(value) => Ok(*value),
        BoolSetting::Integer(value) => Ok(*value != 0),
        BoolSetting::Text(text) => {
            let normalized = text.trim().to_lowercase();
            if TRUE_VALUES.contains(&normalized.as_str()) {
                return Ok(true);
            }
            if FALSE_VALUES.contains(&normalized.as_str()) {
                return Ok(false);
            }
            Err(PillarError::BoolValue {
                value: text.clone(),
            })
        }
    }
}

pub fn resolve_bool_config(
    provided: Option<&BoolSetting>,
    env_var: Option<&str>,
    default: Option<bool>,
    environment: &dyn EnvironmentLookup,
    setting_name: &str,
) -> Option<bool> {
    if let Some(value) = provided {
        if let Ok(parsed) = parse_bool_value(value) {
            return Some(parsed);
        }
        let text = describe(value);
        tracing::warn!(
            "Pillar Guardrail: Invalid boolean value '{text}' for {setting_name}, falling back to default."
        );
        return default;
    }
    let Some(name) = env_var else {
        return default;
    };
    // an empty environment variable counts as unset, as it does for every other typed setting
    let Some(text) = environment.lookup(name).filter(|text| !text.is_empty()) else {
        return default;
    };
    let value = BoolSetting::Text(text.clone());
    if let Ok(parsed) = parse_bool_value(&value) {
        return Some(parsed);
    }
    tracing::warn!(
        "Pillar Guardrail: Invalid boolean env value '{text}' for {name}, falling back to default."
    );
    default
}

fn describe(value: &BoolSetting) -> String {
    match value {
        BoolSetting::Boolean(value) => value.to_string(),
        BoolSetting::Integer(value) => value.to_string(),
        BoolSetting::Text(text) => text.clone(),
    }
}

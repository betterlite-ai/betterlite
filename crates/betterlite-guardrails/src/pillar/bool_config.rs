use betterlite_config::loader::EnvironmentLookup;

use super::error::PillarError;

// LiteLLM's _parse_bool_value takes bool, int or str, and the resolver's provided value can be any of them
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoolSetting {
    Boolean(bool),
    Integer(i64),
    Text(String),
}

pub fn parse_flag_setting(value: &BoolSetting) -> Result<bool, PillarError> {
    match value {
        BoolSetting::Boolean(flag) => Ok(*flag),
        BoolSetting::Integer(number) => Ok(*number != 0),
        BoolSetting::Text(text) => flag_from_text(text),
    }
}

// betterlite-env reads the shared words; the Pillar hook also takes a bare y or n
fn flag_from_text(text: &str) -> Result<bool, PillarError> {
    if let Ok(Some(flag)) = betterlite_env::flag("", text) {
        return Ok(flag);
    }
    match text.trim().to_lowercase().as_str() {
        "y" => Ok(true),
        "n" => Ok(false),
        _ => Err(PillarError::BoolValue {
            value: text.to_owned(),
        }),
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
        if let Ok(parsed) = parse_flag_setting(value) {
            return Some(parsed);
        }
        let text = match value {
            BoolSetting::Boolean(flag) => flag.to_string(),
            BoolSetting::Integer(number) => number.to_string(),
            BoolSetting::Text(text) => text.clone(),
        };
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
    if let Ok(parsed) = parse_flag_setting(&value) {
        return Some(parsed);
    }
    tracing::warn!(
        "Pillar Guardrail: Invalid boolean env value '{text}' for {name}, falling back to default."
    );
    default
}

use super::python_repr::python_repr;

const TRUTHY_TOKEN_AUTH_VALUES: [&str; 6] = ["1", "on", "t", "true", "y", "yes"];
const FALSY_TOKEN_AUTH_VALUES: [&str; 7] = ["", "0", "f", "false", "n", "no", "off"];

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum FlagValue {
    Bool(bool),
    Text(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TokenFlagError {
    #[error("{message}")]
    NotRecognized { message: String },
}

pub fn token_auth_flag_enabled(
    value: Option<&FlagValue>,
    env_var: &str,
) -> Result<bool, TokenFlagError> {
    match value {
        None => Ok(false),
        Some(FlagValue::Bool(flag)) => Ok(*flag),
        Some(FlagValue::Text(text)) => {
            let normalized = text.trim().to_lowercase();
            if TRUTHY_TOKEN_AUTH_VALUES.contains(&normalized.as_str()) {
                return Ok(true);
            }
            if FALSY_TOKEN_AUTH_VALUES.contains(&normalized.as_str()) {
                return Ok(false);
            }
            Err(not_recognized(env_var, text))
        }
    }
}

fn not_recognized(env_var: &str, raw: &str) -> TokenFlagError {
    let truthy = TRUTHY_TOKEN_AUTH_VALUES.join(", ");
    let falsy = FALSY_TOKEN_AUTH_VALUES
        .iter()
        .filter(|value| !value.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    TokenFlagError::NotRecognized {
        message: format!(
            "{env_var}={} is not a recognized boolean. Set it to one of {truthy} to turn it on, or to one of {falsy} to turn it off.",
            python_repr(raw)
        ),
    }
}

// Deliberately permissive and distinct from token_auth_flag_enabled: no stripping, no "on".
#[must_use]
pub fn str_to_bool(value: Option<&str>) -> bool {
    let Some(text) = value else { return false };
    matches!(
        text.to_lowercase().as_str(),
        "true" | "1" | "t" | "y" | "yes"
    )
}

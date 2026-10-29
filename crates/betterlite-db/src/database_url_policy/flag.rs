use std::fmt::Write as _;

const TRUTHY_TOKEN_AUTH_VALUES: [&str; 6] = ["1", "on", "t", "true", "y", "yes"];
const FALSY_TOKEN_AUTH_VALUES: [&str; 7] = ["", "0", "f", "false", "n", "no", "off"];

/// `token_auth_flag_enabled`'s `value: str | bool | None` parameter.
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

/// `token_auth_flag_enabled`: booleans pass through, `None` maps to `false`,
/// strings are stripped and lower-cased before matching either spelling list.
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

/// Python's `repr()` for the realistic range of a flag's raw string value.
fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if character == quote => {
                out.push('\\');
                out.push(quote);
            }
            _ if u32::from(character) < 0x20 || u32::from(character) == 0x7f => {
                if write!(out, "\\x{:02x}", u32::from(character)).is_err() {
                    return out;
                }
            }
            _ => out.push(character),
        }
    }
    out.push(quote);
    out
}

/// Extras `str_to_bool`: deliberately permissive and distinct from
/// `token_auth_flag_enabled`, with no stripping and no `"on"` spelling.
#[must_use]
pub fn str_to_bool(value: Option<&str>) -> bool {
    value.is_some_and(|text| {
        matches!(
            text.to_lowercase().as_str(),
            "true" | "1" | "t" | "y" | "yes"
        )
    })
}

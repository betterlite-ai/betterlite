#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum FlagValue {
    Bool(bool),
    Text(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TokenFlagError {
    #[error("{source}")]
    NotRecognized {
        #[source]
        source: betterlite_env::EnvError,
    },
}

pub fn token_auth_flag_enabled(
    value: Option<&FlagValue>,
    env_var: &str,
) -> Result<bool, TokenFlagError> {
    match value {
        None => Ok(false),
        Some(FlagValue::Bool(flag)) => Ok(*flag),
        Some(FlagValue::Text(text)) => betterlite_env::flag(env_var, text)
            .map(|flag| flag.unwrap_or(false))
            .map_err(|source| TokenFlagError::NotRecognized { source }),
    }
}

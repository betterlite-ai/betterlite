#[path = "bedrock_web_identity/tests.rs"]
mod tests;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
pub const DEFAULT_MAX_POLICY_SIZE: i64 = 75;
// this section reads no YAML key; BEDROCK_MAX_POLICY_SIZE comes from the process environment
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub max_policy_size: i64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            max_policy_size: DEFAULT_MAX_POLICY_SIZE,
        }
    }
}
impl Section for Raw {
    type Settings = Settings;
    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let value = lookup.lookup("BEDROCK_MAX_POLICY_SIZE");
        let max_policy_size = match value.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => {
                text.parse::<i64>()
                    .map_err(|source| ConfigError::InvalidInteger {
                        path: "BEDROCK_MAX_POLICY_SIZE".to_owned(),
                        source,
                    })?
            }
            _ => DEFAULT_MAX_POLICY_SIZE,
        };
        Ok(Settings { max_policy_size })
    }
}

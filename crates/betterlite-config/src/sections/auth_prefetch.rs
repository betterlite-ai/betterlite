use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const VARIABLE: &str = "DEFAULT_IN_MEMORY_TTL";
// litellm/constants.py: DEFAULT_IN_MEMORY_TTL = int(os.getenv("DEFAULT_IN_MEMORY_TTL", 5))
const DEFAULT_ORGANIZATION_TTL: f64 = 5.0;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

// only the organization entries read this; membership is fixed at 5 and the registry cap at 30
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub organization_ttl: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            organization_ttl: DEFAULT_ORGANIZATION_TTL,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let organization_ttl = match lookup.lookup(VARIABLE) {
            None => DEFAULT_ORGANIZATION_TTL,
            Some(text) => integer_seconds(&text)?,
        };
        Ok(Settings { organization_ttl })
    }
}

// Python's int() accepts surrounding whitespace and a sign, and fails on an empty or fractional value
fn integer_seconds(text: &str) -> Result<f64, ConfigError> {
    let parsed = text
        .trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: VARIABLE.to_owned(),
            source,
        })?;
    parsed
        .to_string()
        .parse::<f64>()
        .map_err(|source| ConfigError::InvalidFloat {
            path: VARIABLE.to_owned(),
            source,
        })
}

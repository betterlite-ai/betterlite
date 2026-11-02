#[path = "openai_credentials/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";

// a map-shaped Raw, not a unit struct, so serde still accepts (and ignores) the document's keys
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub api_base: String,
    pub organization: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_base: DEFAULT_API_BASE.to_owned(),
            organization: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let api_base = non_empty(lookup.lookup("OPENAI_BASE_URL"))
            .or_else(|| non_empty(lookup.lookup("OPENAI_API_BASE")))
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let organization = non_empty(lookup.lookup("OPENAI_ORGANIZATION"));
        Ok(Settings {
            api_base,
            organization,
        })
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

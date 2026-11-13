#[path = "openai_workload_identity/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

// a map-shaped Raw, not a unit struct, so serde still accepts (and ignores) the document's keys
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub identity_provider_id: Option<String>,
    pub service_account_id: Option<String>,
    pub token_file: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            identity_provider_id: non_empty(lookup.lookup("OPENAI_IDENTITY_PROVIDER_ID")),
            service_account_id: non_empty(lookup.lookup("OPENAI_SERVICE_ACCOUNT_ID")),
            token_file: non_empty(lookup.lookup("OPENAI_IDENTITY_TOKEN_FILE")),
        })
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

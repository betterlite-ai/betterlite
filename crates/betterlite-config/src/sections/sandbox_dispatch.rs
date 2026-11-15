use betterlite_models::decisions::Json;
use betterlite_models::sandbox_dispatch::{SandboxToolEntry, sandbox_tool_entries};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    sandbox_tools: Option<Vec<Json>>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub sandbox_tools: Vec<SandboxToolEntry>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let sandbox_tools = match self.sandbox_tools {
            None => Vec::new(),
            Some(values) => {
                sandbox_tool_entries(values).map_err(|error| ConfigError::InvalidScalar {
                    path: error.path().to_owned(),
                    value: false,
                })?
            }
        };
        Ok(Settings { sandbox_tools })
    }
}

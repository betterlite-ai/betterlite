use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

const MODE_PATH: &str = "general_settings.user_mcp_management_mode";
const VIEW_ALL: &str = "view_all";

// LiteLLM's UserMCPManagementMode: anything but the exact string view_all is restricted
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ManagementMode {
    #[default]
    Restricted,
    ViewAll,
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    user_mcp_management_mode: JsonValue,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub mode: ManagementMode,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self.general_settings.user_mcp_management_mode;
        if raw.is_null() {
            return Ok(Settings {
                mode: ManagementMode::Restricted,
            });
        }
        let Some(text) = raw.as_str() else {
            return Err(ConfigError::InvalidScalar {
                path: MODE_PATH.to_owned(),
                value: false,
            });
        };
        let resolved = resolve_string(text.to_owned(), MODE_PATH, lookup)?;
        let mode = if resolved == VIEW_ALL {
            ManagementMode::ViewAll
        } else {
            ManagementMode::Restricted
        };
        Ok(Settings { mode })
    }
}

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const ENV_NAME: &str = "LITELLM_MCP_CLIENT_SIDE_AUTH_HEADER_NAME";
const DEFAULT_HEADER_NAME: &str = "x-mcp-auth";
const YAML_PATH: &str = "general_settings.mcp_client_side_auth_header_name";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default, rename = "mcp_client_side_auth_header_name")]
    client_side_auth_header: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub header_name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            header_name: DEFAULT_HEADER_NAME.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let resolved = match lookup.lookup(ENV_NAME) {
            Some(value) => value,
            None => match self.general_settings.client_side_auth_header {
                Some(text) => resolve_string(text, YAML_PATH, lookup)?,
                None => DEFAULT_HEADER_NAME.to_owned(),
            },
        };
        Ok(Settings {
            header_name: if resolved.is_empty() {
                DEFAULT_HEADER_NAME.to_owned()
            } else {
                resolved
            },
        })
    }
}

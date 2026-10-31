#[path = "mcp_tool_helpers/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    mcp_client_timeout: Option<String>,
    mcp_tool_listing_timeout: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub client_timeout: f64,
    pub tool_listing_timeout: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            client_timeout: 60.0,
            tool_listing_timeout: 30.0,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let client_timeout = timeout(
            self.litellm_settings.mcp_client_timeout,
            "litellm_settings.mcp_client_timeout",
            "LITELLM_MCP_CLIENT_TIMEOUT",
            lookup,
            60.0,
        )?;
        let tool_listing_timeout = timeout(
            self.litellm_settings.mcp_tool_listing_timeout,
            "litellm_settings.mcp_tool_listing_timeout",
            "LITELLM_MCP_TOOL_LISTING_TIMEOUT",
            lookup,
            30.0,
        )?;
        Ok(Settings {
            client_timeout,
            tool_listing_timeout,
        })
    }
}

fn timeout<L>(
    configured: Option<String>,
    path: &str,
    environment: &str,
    lookup: &L,
    default: f64,
) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let value = configured
        .map(|text| resolve_string(text, path, lookup))
        .transpose()?
        .or_else(|| lookup.lookup(environment))
        .unwrap_or_else(|| default.to_string());
    value
        .parse::<f64>()
        .map_err(|source| ConfigError::InvalidFloat {
            path: path.to_owned(),
            source,
        })
}

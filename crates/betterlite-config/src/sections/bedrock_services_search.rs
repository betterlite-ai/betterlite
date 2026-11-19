use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

pub const GATEWAY_URL_VARIABLE: &str = "AGENTCORE_GATEWAY_URL";
pub const SEARCH_TOOL_NAME_VARIABLE: &str = "AGENTCORE_SEARCH_TOOL_NAME";
pub const MCP_PROTOCOL_VERSION_VARIABLE: &str = "AGENTCORE_MCP_PROTOCOL_VERSION";
pub const GATEWAY_TOKEN_VARIABLE: &str = "AGENTCORE_GATEWAY_TOKEN";
pub const DEFAULT_SEARCH_TOOL_NAME: &str = "web-search-tool___WebSearch";
pub const DEFAULT_MCP_PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<Deployment>,
    #[serde(default)]
    litellm_settings: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    tool_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GatewaySettings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EnvironmentSettings {
    pub gateway_url: Option<String>,
    pub search_tool_name: String,
    pub mcp_protocol_version: String,
    pub gateway_token: Option<SecretString>,
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self {
            gateway_url: None,
            search_tool_name: DEFAULT_SEARCH_TOOL_NAME.to_owned(),
            mcp_protocol_version: DEFAULT_MCP_PROTOCOL_VERSION.to_owned(),
            gateway_token: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub deployments: Box<[GatewaySettings]>,
    pub global: GatewaySettings,
    pub environment: EnvironmentSettings,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let global = resolve_params(self.litellm_settings, "litellm_settings", lookup)?;
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let path = format!("model_list.{index}.litellm_params");
            deployments.push(resolve_params(deployment.litellm_params, &path, lookup)?);
        }
        Ok(Settings {
            deployments: deployments.into_boxed_slice(),
            global,
            environment: EnvironmentSettings {
                gateway_url: present(lookup, GATEWAY_URL_VARIABLE),
                search_tool_name: present(lookup, SEARCH_TOOL_NAME_VARIABLE)
                    .unwrap_or_else(|| DEFAULT_SEARCH_TOOL_NAME.to_owned()),
                mcp_protocol_version: present(lookup, MCP_PROTOCOL_VERSION_VARIABLE)
                    .unwrap_or_else(|| DEFAULT_MCP_PROTOCOL_VERSION.to_owned()),
                gateway_token: present(lookup, GATEWAY_TOKEN_VARIABLE).map(SecretString::from),
            },
        })
    }
}

fn resolve_params<L>(params: Params, path: &str, lookup: &L) -> Result<GatewaySettings, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(GatewaySettings {
        api_base: resolve_text(params.api_base, &format!("{path}.api_base"), lookup)?,
        tool_name: resolve_text(params.tool_name, &format!("{path}.tool_name"), lookup)?,
        api_key: resolve_secret(params.api_key, &format!("{path}.api_key"), lookup)?,
    })
}

fn resolve_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(None),
        Some(text) => Ok(Some(resolve_string(text, path, lookup)?)),
    }
}

// LiteLLM reads these through get_secret_str, where an empty value counts as unset
fn present<L>(lookup: &L, variable: &str) -> Option<String>
where
    L: EnvironmentLookup,
{
    lookup.lookup(variable).filter(|value| !value.is_empty())
}

mod tests;

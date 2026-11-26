use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PROXY_BASE_URL_NAME: &str = "PROXY_BASE_URL";
const TRUSTED_PROXY_RANGES_PATH: &str = "general_settings.trusted_proxy_ranges";
const MCP_TRUSTED_PROXY_RANGES_PATH: &str = "general_settings.mcp_trusted_proxy_ranges";
const MCP_XFF_HOPS_PATH: &str = "general_settings.mcp_xff_num_trusted_hops";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

// ProxyConfig declares these as list[str] | None, so a scalar is a load error rather than a runtime fallback
#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    trusted_proxy_ranges: Option<Vec<String>>,
    #[serde(default)]
    mcp_trusted_proxy_ranges: Option<Vec<String>>,
    #[serde(default)]
    use_x_forwarded_for: Option<bool>,
    #[serde(default)]
    mcp_xff_num_trusted_hops: Option<RawNumber>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub proxy_base_url: Option<String>,
    pub use_x_forwarded_for: bool,
    pub trusted_proxy_ranges: Option<Vec<String>>,
    pub mcp_trusted_proxy_ranges: Option<Vec<String>>,
    pub mcp_xff_num_trusted_hops: Option<i64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general_settings = self.general_settings;
        Ok(Settings {
            // is_request_https reads the variable with an empty default and trims it before parsing
            proxy_base_url: lookup.lookup(PROXY_BASE_URL_NAME),
            use_x_forwarded_for: general_settings.use_x_forwarded_for.unwrap_or_default(),
            trusted_proxy_ranges: ranges(
                general_settings.trusted_proxy_ranges,
                TRUSTED_PROXY_RANGES_PATH,
                lookup,
            )?,
            mcp_trusted_proxy_ranges: ranges(
                general_settings.mcp_trusted_proxy_ranges,
                MCP_TRUSTED_PROXY_RANGES_PATH,
                lookup,
            )?,
            mcp_xff_num_trusted_hops: hops(general_settings.mcp_xff_num_trusted_hops, lookup)?,
        })
    }
}

// LiteLLM substitutes os.environ/ references anywhere in the config, including a list element
fn ranges<L>(
    configured: Option<Vec<String>>,
    path: &str,
    lookup: &L,
) -> Result<Option<Vec<String>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(configured) = configured else {
        return Ok(None);
    };
    let mut resolved = Vec::with_capacity(configured.len());
    for (index, range) in configured.into_iter().enumerate() {
        resolved.push(resolve_string(range, &format!("{path}[{index}]"), lookup)?);
    }
    Ok(Some(resolved))
}

// ProxyConfig declares ge=1, so a zero or negative count refuses the config instead of failing closed later
fn hops<L>(configured: Option<RawNumber>, lookup: &L) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(value) = resolve_number(configured, MCP_XFF_HOPS_PATH, lookup, parse_i64)? else {
        return Ok(None);
    };
    if value < 1 {
        return Err(ConfigError::InvalidScalar {
            path: MCP_XFF_HOPS_PATH.to_owned(),
            value: false,
        });
    }
    Ok(Some(value))
}

fn parse_i64(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

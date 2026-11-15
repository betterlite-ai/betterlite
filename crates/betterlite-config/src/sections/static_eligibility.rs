use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    #[serde(default)]
    enable_pre_call_checks: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

// enable_pre_call_checks is the switch over the context-window and supported-parameter filters; it defaults off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub enable_pre_call_checks: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            enable_pre_call_checks: configured_flag(
                self.router_settings.enable_pre_call_checks,
                "router_settings.enable_pre_call_checks",
                lookup,
            )?,
        })
    }
}

// ProxyConfig resolves os.environ/ strings in router_settings before Router init; the text is then read as a flag.
fn configured_flag<L>(value: Option<RawFlag>, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(false),
        Some(RawFlag::Boolean(flag)) => Ok(flag),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix("os.environ/").unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
    }
}

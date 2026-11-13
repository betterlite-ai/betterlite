use serde::{Deserialize, de::IgnoredAny};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

#[path = "weighted_failover/tests.rs"]
mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    enable_weighted_failover: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = "router_settings.enable_weighted_failover";
        let enabled = match self.router_settings.enable_weighted_failover {
            None => false,
            Some(RawFlag::Boolean(flag)) => flag,
            Some(RawFlag::Text(text)) => match text.strip_prefix("os.environ/") {
                Some(reference) => {
                    let variable = reference.to_owned();
                    let resolved = resolve_string(text, path, lookup)?;
                    betterlite_env::flag(&variable, &resolved)?.unwrap_or(false)
                }
                None => {
                    return Err(ConfigError::InvalidScalar {
                        path: path.to_owned(),
                        value: false,
                    });
                }
            },
            Some(RawFlag::Other(_)) => {
                return Err(ConfigError::InvalidScalar {
                    path: path.to_owned(),
                    value: false,
                });
            }
        };
        Ok(Settings { enabled })
    }
}

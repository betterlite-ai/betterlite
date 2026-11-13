use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const IGNORE_PATH: &str = "router_settings.ignore_invalid_deployments";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    #[serde(default)]
    ignore_invalid_deployments: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // skip a deployment that fails to build instead of failing the whole model list
    pub ignore_invalid_deployments: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let ignore_invalid_deployments = match self.router_settings.ignore_invalid_deployments {
            None => false,
            Some(RawFlag::Boolean(flag)) => flag,
            Some(RawFlag::Text(text)) => {
                let name = text
                    .strip_prefix("os.environ/")
                    .unwrap_or(IGNORE_PATH)
                    .to_owned();
                let resolved = resolve_string(text, IGNORE_PATH, lookup)?;
                betterlite_env::flag(&name, &resolved)?.unwrap_or(false)
            }
        };
        Ok(Settings {
            ignore_invalid_deployments,
        })
    }
}

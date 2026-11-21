use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_integer, resolve_number};
use crate::sections::Section;

mod tests;

const TTL_PATH: &str = "router_settings.deployment_affinity_ttl_seconds";
// Router.deployment_affinity_ttl_seconds's declaration default
const DEFAULT_TTL_SECONDS: u32 = 3600;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    optional_pre_call_checks: Option<Vec<String>>,
    model_group_affinity_config: Option<IndexMap<String, Vec<String>>>,
    deployment_affinity_ttl_seconds: Option<RawNumber>,
}

// add_optional_pre_call_checks and _ensure_deployment_affinity_callback read these three keys
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub optional_pre_call_checks: Option<Vec<String>>,
    pub model_group_affinity_config: Option<IndexMap<String, Vec<String>>>,
    pub deployment_affinity_ttl_seconds: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            optional_pre_call_checks: None,
            model_group_affinity_config: None,
            deployment_affinity_ttl_seconds: DEFAULT_TTL_SECONDS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let ttl = resolve_number(
            self.router_settings.deployment_affinity_ttl_seconds,
            TTL_PATH,
            lookup,
            parse_integer,
        )?;
        Ok(Settings {
            optional_pre_call_checks: self.router_settings.optional_pre_call_checks,
            model_group_affinity_config: self.router_settings.model_group_affinity_config,
            deployment_affinity_ttl_seconds: ttl.unwrap_or(DEFAULT_TTL_SECONDS),
        })
    }
}

use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub general_settings: RawGeneral,
}

// absent and null both deserialize to None, and both take LiteLLM's False default
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawGeneral {
    #[serde(default, rename = "disable_agents_for_internal_users")]
    agents_disabled: Option<RawSwitch>,
    #[serde(default, rename = "allow_agents_for_team_admins")]
    agents_for_team_admins: Option<RawSwitch>,
    #[serde(default, rename = "disable_vector_stores_for_internal_users")]
    vector_stores_disabled: Option<RawSwitch>,
    #[serde(default, rename = "allow_vector_stores_for_team_admins")]
    vector_stores_for_team_admins: Option<RawSwitch>,
}

// the guard reads these keys from the open general_settings map, so any YAML value can sit under them
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawSwitch {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    List(Vec<IgnoredAny>),
    Mapping(IndexMap<String, IgnoredAny>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeatureSwitches {
    pub disabled: bool,
    pub team_admins_allowed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub agents: FeatureSwitches,
    pub vector_stores: FeatureSwitches,
}

// rbac_utils.py tests general_settings.get(flag, False) for truthiness, so a quoted "false" still switches on
impl RawSwitch {
    fn truthy(self, lookup: &dyn EnvironmentLookup) -> Result<bool, ConfigError> {
        Ok(match self {
            Self::Boolean(value) => value,
            Self::Integer(number) => number != 0,
            Self::Float(number) => number != 0.0_f64,
            Self::Text(text) => match text.strip_prefix("os.environ/") {
                // a referenced variable is read by the shared env parser, and unset or empty stays off
                Some(variable) => {
                    let resolved = lookup.lookup(variable).unwrap_or_default();
                    betterlite_env::flag(variable, &resolved)?.unwrap_or(false)
                }
                None => !text.is_empty(),
            },
            Self::List(items) => !items.is_empty(),
            Self::Mapping(entries) => !entries.is_empty(),
        })
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let state = |switch: Option<RawSwitch>| switch.map_or(Ok(false), |raw| raw.truthy(lookup));
        Ok(Settings {
            agents: FeatureSwitches {
                disabled: state(general.agents_disabled)?,
                team_admins_allowed: state(general.agents_for_team_admins)?,
            },
            vector_stores: FeatureSwitches {
                disabled: state(general.vector_stores_disabled)?,
                team_admins_allowed: state(general.vector_stores_for_team_admins)?,
            },
        })
    }
}

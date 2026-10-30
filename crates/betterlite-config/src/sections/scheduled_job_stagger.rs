use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const DEFAULT_WINDOW_SECONDS: u32 = 300;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    scheduled_job_stagger: Option<RawStaggerNode>,
}

// one field in the wrong shape falls back to IgnoredAny (a storable serde_json::Value substitute) instead of invalidating the whole document
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawStaggerNode {
    Fields(StaggerFields),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct StaggerFields {
    enabled: bool,
    window_seconds: u32,
    identity: Option<String>,
    offsets: BTreeMap<String, i64>,
}

impl Default for StaggerFields {
    fn default() -> Self {
        Self {
            enabled: true,
            window_seconds: DEFAULT_WINDOW_SECONDS,
            identity: None,
            offsets: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub window_seconds: u32,
    pub identity: Option<String>,
    pub offsets: BTreeMap<String, i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from(StaggerFields::default())
    }
}

impl From<StaggerFields> for Settings {
    fn from(fields: StaggerFields) -> Self {
        Self {
            enabled: fields.enabled,
            window_seconds: fields.window_seconds,
            identity: fields.identity,
            offsets: fields.offsets,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(match self.general_settings.scheduled_job_stagger {
            None => Settings::default(),
            Some(RawStaggerNode::Fields(fields)) => Settings::from(fields),
            Some(RawStaggerNode::Other(_)) => {
                tracing::warn!(
                    "ignoring invalid general_settings.scheduled_job_stagger, falling back to defaults"
                );
                Settings::default()
            }
        })
    }
}

mod defaults;
mod invalid;

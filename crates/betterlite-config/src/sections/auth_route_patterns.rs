#[path = "auth_route_patterns/tests.rs"]
mod tests;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    public_routes: Vec<RawRoute>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub public_routes: Vec<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            public_routes: self
                .general_settings
                .public_routes
                .into_iter()
                .map(|RawRoute::String(route)| route)
                .collect(),
        })
    }
}

// Untagged content preserves scalar types instead of coercing YAML numbers to strings.
#[derive(Debug, Deserialize)]
#[serde(untagged, expecting = "a string in general_settings.public_routes")]
enum RawRoute {
    String(String),
}

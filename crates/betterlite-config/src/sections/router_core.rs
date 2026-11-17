use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::types::RoutingStrategy;

mod tests;

// Router._normalize_strategy reads router_settings.routing_strategy, and Router.__init__ defaults it to simple-shuffle.
const DEFAULT_ROUTING_STRATEGY: RoutingStrategy = RoutingStrategy::SimpleShuffle;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    #[serde(default)]
    routing_strategy: Option<RoutingStrategy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub routing_strategy: RoutingStrategy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            routing_strategy: DEFAULT_ROUTING_STRATEGY,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            routing_strategy: self
                .router_settings
                .routing_strategy
                .unwrap_or(DEFAULT_ROUTING_STRATEGY),
        })
    }
}

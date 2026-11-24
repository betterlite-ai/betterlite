use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::auth_prefetch;

mod tests;

// DEFAULT_IN_MEMORY_TTL belongs to the shared prefetch loader, so the TTL is read through that section
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub in_memory_ttl: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            in_memory_ttl: auth_prefetch::Settings::default().organization_ttl,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let shared = auth_prefetch::Raw::default().convert(lookup)?;
        Ok(Settings {
            in_memory_ttl: shared.organization_ttl,
        })
    }
}

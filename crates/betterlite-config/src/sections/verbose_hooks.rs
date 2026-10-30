#[path = "verbose_hooks/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Global {
    pub set_verbose: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub set_verbose: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let set_verbose = self.litellm_settings.set_verbose.unwrap_or_else(|| {
            lookup
                .lookup("LITELLM_SET_VERBOSE")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        });
        Ok(Settings { set_verbose })
    }
}

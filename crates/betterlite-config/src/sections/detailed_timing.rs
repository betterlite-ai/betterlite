use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // LiteLLM reads this flag only from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetailedTiming {
    pub enabled: bool,
}

impl Section for Raw {
    type Settings = DetailedTiming;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM lowercases the value and compares it with "true", so "1", "yes" and a padded "true" stay off
        let enabled = lookup
            .lookup("LITELLM_DETAILED_TIMING")
            .is_some_and(|text| text.to_lowercase() == "true");
        Ok(DetailedTiming { enabled })
    }
}

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

// Both values only expand Windows paths for the Azure Arc agent; an empty value counts as unset.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub program_data: Option<String>,
    pub program_files: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            program_data: lookup
                .lookup("PROGRAMDATA")
                .filter(|value| !value.is_empty()),
            program_files: lookup
                .lookup("ProgramFiles")
                .filter(|value| !value.is_empty()),
        })
    }
}

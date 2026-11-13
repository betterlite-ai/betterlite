use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const UNEXPANDED_PROGRAM_DATA: &str = "%ProgramData%";
const UNEXPANDED_PROGRAM_FILES: &str = "${ProgramFiles}";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

// Each name keeps the literal form MSAL writes it in: ntpath.expandvars leaves an unset ${ProgramFiles} or %ProgramData% untouched.
#[derive(Debug, Clone)]
pub struct Settings {
    pub program_data: String,
    pub program_files: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            program_data: UNEXPANDED_PROGRAM_DATA.to_owned(),
            program_files: UNEXPANDED_PROGRAM_FILES.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let defaults = Settings::default();
        Ok(Settings {
            program_data: lookup
                .lookup("PROGRAMDATA")
                .unwrap_or(defaults.program_data),
            program_files: lookup
                .lookup("ProgramFiles")
                .unwrap_or(defaults.program_files),
        })
    }
}

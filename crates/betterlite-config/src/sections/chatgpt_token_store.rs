use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::providers::chatgpt::token_store as environment;
use crate::sections::Section;

pub mod home;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub token_dir: String,
    pub auth_file: String,
    pub api_base: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let paths = environment::resolve(lookup);
        Ok(Settings {
            token_dir: paths.token_dir,
            auth_file: paths.auth_file,
            api_base: paths.api_base,
        })
    }
}

mod tests;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    pass_through_request_timeout: Option<RawNumber>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Settings {
    pub request_timeout: Option<f64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let request_timeout = resolve_number(
            self.general_settings.pass_through_request_timeout,
            "general_settings.pass_through_request_timeout",
            lookup,
            parse_float,
        )?;
        Ok(Settings { request_timeout })
    }
}

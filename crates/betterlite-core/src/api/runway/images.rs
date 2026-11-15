pub mod cost;
pub mod error;
pub mod params;
pub mod reply;
pub mod transform;

mod tests;

use betterlite_config::loader::EnvironmentLookup;

use self::error::RunwayImageError;
use crate::api::runway::tasks::settings::RunwaySettings;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["size"];
pub const DEFAULT_MODEL: &str = "gen4_image";
pub const DEFAULT_RATIO: &str = "1920:1080";

#[derive(Debug, Clone)]
pub struct RunwayImageConfig {
    pub settings: RunwaySettings,
}

impl RunwayImageConfig {
    // LiteLLM's selector answers this one configuration whatever model name it is handed
    pub fn for_model(
        _model: &str,
        lookup: &dyn EnvironmentLookup,
    ) -> Result<Self, RunwayImageError> {
        Ok(Self {
            settings: RunwaySettings::load(lookup)?,
        })
    }
}

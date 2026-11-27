pub mod dispatch;
pub mod error;
pub mod mapping;
pub mod request;
pub mod response;

mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;

use self::error::RunwaySpeechError;
use crate::api::runway::tasks::settings::RunwaySettings;
use crate::api::speech_call::operations::{SpeechConfigOperation, SpeechMappingOptions};

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["voice"];
pub const DEFAULT_MODEL: &str = "eleven_multilingual_v2";
pub const DEFAULT_VOICE_TYPE: &str = "runway-preset";
pub const DEFAULT_VOICE_PRESET_ID: &str = "Bernard";

#[derive(Debug, Clone)]
pub struct RunwaySpeechConfig {
    pub settings: RunwaySettings,
}

impl RunwaySpeechConfig {
    // LiteLLM's selector answers this one configuration whatever model name it is handed
    pub fn for_model(
        _model: &str,
        lookup: &dyn EnvironmentLookup,
    ) -> Result<Self, RunwaySpeechError> {
        Ok(Self {
            settings: RunwaySettings::load(lookup)?,
        })
    }
}

impl SpeechConfigOperation for RunwaySpeechConfig {
    fn map_openai_params(
        &self,
        _model: &str,
        _optional_params: Object,
        options: &SpeechMappingOptions,
    ) -> (Option<SpeechVoice>, Object) {
        mapping::map_openai_params(options.voice.as_ref())
    }
}

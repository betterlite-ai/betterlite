use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;

use crate::api::speech_call::operations::{SpeechConfigOperation, SpeechMappingOptions};

use super::mapping::map_openai_params;

// the AWSPollyTextToSpeechConfig B1135 selects by tag; the request side lives in this module's other files
pub struct PollySpeechConfig;

impl SpeechConfigOperation for PollySpeechConfig {
    fn map_openai_params(
        &self,
        model: &str,
        optional_params: Object,
        options: &SpeechMappingOptions,
    ) -> (Option<SpeechVoice>, Object) {
        map_openai_params(model, &optional_params, options)
    }
}

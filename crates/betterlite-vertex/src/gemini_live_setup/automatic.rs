use betterlite_prompts::arguments::json::Object;

use super::catalog::default_response_modality;
use super::error::LiveSetupError;
use super::settings::LiveSetupSettings;
use super::wire::{Envelope, GenerationConfig, LiveSetup};

#[must_use]
pub fn automatic_setup(model: &str, settings: &LiveSetupSettings<'_>) -> LiveSetup {
    LiveSetup {
        model: Some(format!("models/{model}")),
        generation_config: GenerationConfig {
            temperature: None,
            max_output_tokens: None,
            response_modalities: Some(vec![
                default_response_modality(settings.catalog, model).to_owned(),
            ]),
            speech_config: None,
        },
        system_instruction: None,
        tools: None,
        tool_config: None,
        realtime_input_config: None,
        session_resumption: None,
        session_resumption_config: None,
        context_window_compression: None,
        input_audio_transcription: Some(Object::new()),
        output_audio_transcription: None,
    }
}

pub fn session_configuration_request(
    model: &str,
    settings: &LiveSetupSettings<'_>,
) -> Result<Option<String>, LiveSetupError> {
    if settings.defer_setup {
        return Ok(None);
    }
    let setup = automatic_setup(model, settings);
    Ok(Some(
        serde_json::to_string(&Envelope { setup: &setup }).map_err(LiveSetupError::Encode)?,
    ))
}

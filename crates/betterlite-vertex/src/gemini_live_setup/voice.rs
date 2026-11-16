use betterlite_prompts::arguments::json::Object;

use crate::gemini_thinking::audio::{AudioParam, map_audio_params};
use crate::gemini_thinking::error::GeminiThinkingError;

const OPENAI_STOCK_REALTIME_VOICES: [&str; 10] = [
    "alloy", "ash", "ballad", "cedar", "coral", "echo", "marin", "sage", "shimmer", "verse",
];

#[derive(Debug, Eq, PartialEq)]
pub enum SpeechSelection {
    Voice(Object),
    Dropped,
}

pub fn speech_config(voice: &str) -> Result<SpeechSelection, GeminiThinkingError> {
    if OPENAI_STOCK_REALTIME_VOICES.contains(&voice.to_lowercase().as_str()) {
        tracing::warn!(
            "Gemini Realtime: voice {voice} is an OpenAI voice with no Gemini equivalent; dropping it so the session keeps the model's default voice."
        );
        return Ok(SpeechSelection::Dropped);
    }
    let config = map_audio_params(&AudioParam {
        format: None,
        voice: Some(voice.to_owned()),
        language_code: None,
    })?;
    Ok(SpeechSelection::Voice(config))
}

use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use serde_json::json;

use super::DEFAULT_VOICE_TYPE;

pub const VOICE_KEY: &str = "runwayml_voice";

const VOICE_MAPPINGS: [(&str, &str); 6] = [
    ("alloy", "Maya"),
    ("echo", "James"),
    ("fable", "Bernard"),
    ("onyx", "Vincent"),
    ("nova", "Serene"),
    ("shimmer", "Ella"),
];

// LiteLLM maps only voice: every option that reached it, and drop_params itself, leaves no trace
#[must_use]
pub fn map_openai_params(voice: Option<&SpeechVoice>) -> (Option<SpeechVoice>, Object) {
    let Some(voice) = voice else {
        return (None, Object::new());
    };
    let mapped = match voice {
        SpeechVoice::Name(name) => json!({
            "type": DEFAULT_VOICE_TYPE,
            "presetId": preset_id(name),
        }),
        SpeechVoice::Options(fields) => json!(fields),
    };
    let mut params = Object::new();
    params.insert(VOICE_KEY.to_owned(), mapped);
    (None, params)
}

fn preset_id(name: &str) -> &str {
    VOICE_MAPPINGS
        .iter()
        .find(|(openai, _)| *openai == name)
        .map_or(name, |(_, preset)| *preset)
}

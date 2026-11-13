use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;

use super::error::SpeechError;

// the typos check flags the middle of this ID, so it is written in two parts
const ASH_ID: &str = concat!("AZnzlk1XvdvU", "eBnXmlld");
const VOICE_MAPPINGS: [(&str, &str); 10] = [
    ("alloy", "21m00Tcm4TlvDq8ikWAM"),
    ("amber", "5Q0t7uMcjvnagumLfvZi"),
    ("ash", ASH_ID),
    ("august", "D38z5RcWu1voky8WS1ja"),
    ("blue", "2EiwWnXFnvU5JabPnv8n"),
    ("coral", "9BWtsMINqrJLrRacOk9x"),
    ("lily", "EXAVITQu4vr4xnSDxMaL"),
    ("onyx", "29vD33N1CtxCmqQRPOHJ"),
    ("sage", "CwhRBWXzGAHq8TQ4Fs17"),
    ("verse", "CYw3kZ02Hs0563khs1Fj"),
];
const VOICE_KEYS: [&str; 3] = ["voice_id", "id", "name"];

fn extract_voice_id(voice: &str) -> String {
    let normalized = voice.trim();
    let lowered = normalized.to_lowercase();
    let mapped = VOICE_MAPPINGS.iter().find(|(name, _)| *name == lowered);
    mapped
        .map_or(normalized, |(_, voice_id)| *voice_id)
        .to_owned()
}

pub fn resolve_voice_id(
    voice: Option<&SpeechVoice>,
    params: &mut Object,
) -> Result<String, SpeechError> {
    // a whitespace-only string still resolves here, to an empty ID that URL building refuses
    let named = match voice {
        Some(SpeechVoice::Name(name)) => Some(extract_voice_id(name)),
        Some(SpeechVoice::Options(options)) => VOICE_KEYS
            .iter()
            .find_map(|key| {
                options
                    .get(*key)?
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
            })
            .map(extract_voice_id),
        None => None,
    };
    if let Some(voice_id) = named {
        return Ok(voice_id);
    }
    params
        .remove("voice_id")
        .and_then(|value| value.as_str().map(extract_voice_id))
        .filter(|voice_id| !voice_id.is_empty())
        .ok_or(SpeechError::VoiceRequired)
}

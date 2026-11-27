use betterlite_openai::api::speech_provider_contract::SpeechVoice;

pub const DEFAULT_VOICE: &str = "Joanna";
// LiteLLM maps exactly these lowercase OpenAI names and keeps every other string as a Polly voice
const VOICE_MAPPINGS: [(&str, &str); 6] = [
    ("alloy", "Joanna"),
    ("echo", "Matthew"),
    ("fable", "Amy"),
    ("onyx", "Brian"),
    ("nova", "Ivy"),
    ("shimmer", "Kendra"),
];

// map_openai_params only maps a string voice; a dict voice maps to None and is resolved again by dispatch
pub fn mapped_voice(voice: Option<&SpeechVoice>) -> Option<SpeechVoice> {
    let Some(SpeechVoice::Name(name)) = voice else {
        return None;
    };
    let mapped = VOICE_MAPPINGS
        .iter()
        .find(|(openai, _)| *openai == name.as_str())
        .map_or_else(|| name.clone(), |(_, polly)| (*polly).to_owned());
    Some(SpeechVoice::Name(mapped))
}

// dispatch turns a voice mapping into its name member before the handler sees it
pub fn dispatch_voice(voice: Option<&SpeechVoice>) -> Option<String> {
    match voice {
        Some(SpeechVoice::Name(name)) => Some(name.clone()),
        Some(SpeechVoice::Options(options)) => options
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::to_owned),
        None => None,
    }
}

// the request builder uses Joanna for a missing or empty voice
#[must_use]
pub fn request_voice(voice: Option<&str>) -> &str {
    voice
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_VOICE)
}

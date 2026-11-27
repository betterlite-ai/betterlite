use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use serde_json::json;

use crate::api::speech_call::operations::SpeechMappingOptions;

use super::voice::mapped_voice;

pub const DEFAULT_ENGINE: &str = "neural";
pub const DEFAULT_OUTPUT_FORMAT: &str = "mp3";
const VALID_ENGINES: [&str; 4] = ["standard", "neural", "long-form", "generative"];
const FORMAT_MAPPINGS: [(&str, &str); 6] = [
    ("mp3", "mp3"),
    ("opus", "ogg_vorbis"),
    ("aac", "mp3"),
    ("flac", "mp3"),
    ("wav", "pcm"),
    ("pcm", "pcm"),
];
const POLLY_KWARGS: [(&str, &str); 3] = [
    ("language_code", "LanguageCode"),
    ("lexicon_names", "LexiconNames"),
    ("sample_rate", "SampleRate"),
];

// only the second slash component names an engine, and a bare standard model still selects neural
#[must_use]
pub fn extract_engine(model: &str) -> &'static str {
    let mut parts = model.split('/');
    let _leading = parts.next();
    let Some(component) = parts.next() else {
        return DEFAULT_ENGINE;
    };
    let lowered = component.to_lowercase();
    VALID_ENGINES
        .iter()
        .copied()
        .find(|engine| *engine == lowered.as_str())
        .unwrap_or(DEFAULT_ENGINE)
}

// LiteLLM's map_openai_params: the voice, then the format, the engine and the Polly kwargs in their AWS casing
#[must_use]
pub fn map_openai_params(
    model: &str,
    optional_params: &Object,
    options: &SpeechMappingOptions,
) -> (Option<SpeechVoice>, Object) {
    let mut mapped = Object::new();
    let mut format = json!(DEFAULT_OUTPUT_FORMAT);
    let requested = optional_params
        .get("response_format")
        .filter(|value| !value.is_null());
    if let Some(value) = requested {
        format = match value.as_str() {
            Some(name) => FORMAT_MAPPINGS
                .iter()
                .find(|(openai, _)| *openai == name)
                .map_or_else(|| value.clone(), |(_, mapped)| json!(*mapped)),
            None => value.clone(),
        };
    }
    mapped.insert(String::from("output_format"), format);
    mapped.insert(String::from("engine"), json!(extract_engine(model)));
    for (source, target) in POLLY_KWARGS {
        if let Some(value) = options.kwargs.get(source) {
            mapped.insert(target.to_owned(), value.clone());
        }
    }
    (mapped_voice(options.voice.as_ref()), mapped)
}

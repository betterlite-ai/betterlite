use super::error::MistralAudioError;
use super::reply::reply_fields;
use crate::api::speech_provider_contract::{SpeechVoice, TextToSpeechRequestData};
use crate::files::records::BinaryContent;
use crate::files::stream::flatten_headers;
use crate::openai_base_config::Object;
use base64::Engine as _;
use base64::alphabet;
use base64::engine::{GeneralPurpose, GeneralPurposeConfig};
use betterlite_config::sections::mistral_resources::Settings;
use betterlite_http::proxy::UpstreamResponse;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize as _;
use serde_json::Value;

pub const DEFAULT_BASE: &str = "https://api.mistral.ai/v1";
pub const OPENAI_PARAMS: [&str; 2] = ["voice", "response_format"];
fn alias(voice: &str) -> Option<&'static str> {
    match voice {
        "alloy" => Some("en_paul_neutral"),
        "echo" => Some("gb_oliver_neutral"),
        "fable" => Some("en_paul_cheerful"),
        "onyx" => Some("en_paul_confident"),
        "nova" | "shimmer" => Some("gb_jane_sarcasm"),
        _ => None,
    }
}
fn mime(format: Option<&str>) -> &'static str {
    match format {
        Some("wav") => "audio/wav",
        Some("pcm") => "audio/pcm",
        Some("flac") => "audio/flac",
        Some("opus") => "audio/ogg",
        _ => "audio/mpeg",
    }
}
// Python's b64decode(validate=True) rejects stray characters and bad padding but keeps nonzero trailing bits
const AUDIO_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

#[must_use]
pub fn map_openai_params(
    optional_params: &Object,
    voice: Option<&SpeechVoice>,
    kwargs: &Object,
) -> (Option<String>, Object) {
    let kwarg_voice = kwargs
        .get("voice_id")
        .and_then(|value| SpeechVoice::deserialize(value).ok());
    let mapped_voice = resolve_voice(voice).or_else(|| resolve_voice(kwarg_voice.as_ref()));
    let mut mapped_params = Object::new();
    for name in ["response_format", "ref_audio"] {
        if let Some(Value::String(text)) = optional_params.get(name) {
            mapped_params.insert((*name).to_owned(), Value::String(text.clone()));
        }
    }
    (mapped_voice, mapped_params)
}
fn resolve_voice(voice: Option<&SpeechVoice>) -> Option<String> {
    let text = match voice? {
        SpeechVoice::Name(name) => name.trim(),
        SpeechVoice::Options(options) => ["voice_id", "id", "name"].iter().find_map(|name| {
            options
                .get(*name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })?,
    };
    if text.is_empty() {
        return None;
    }
    let lowered = text.to_lowercase();
    Some(alias(&lowered).map_or_else(|| text.to_owned(), str::to_owned))
}
pub fn validate_environment(
    headers: &IndexMap<String, String>,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<IndexMap<String, String>, MistralAudioError> {
    let key = api_key
        .or(settings.api_key.as_ref())
        .ok_or(MistralAudioError::MissingApiKey)?;
    let mut merged = headers.clone();
    let bearer = format!("Bearer {}", key.expose_secret());
    merged.insert(String::from("Authorization"), bearer);
    merged.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    Ok(merged)
}
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base.unwrap_or(DEFAULT_BASE).trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/audio/speech")
    } else {
        format!("{base}/v1/audio/speech")
    }
}
#[must_use]
pub fn request_body(
    model: &str,
    input: &str,
    voice: Option<&str>,
    optional_params: &Object,
) -> TextToSpeechRequestData {
    let mut body = Object::new();
    body.insert(String::from("model"), Value::String(model.to_owned()));
    body.insert(String::from("input"), Value::String(input.to_owned()));
    if let Some(voice_id) = voice {
        body.insert(String::from("voice_id"), Value::String(voice_id.to_owned()));
    }
    for name in ["response_format", "ref_audio"] {
        if let Some(Value::String(text)) = optional_params.get(name) {
            body.insert((*name).to_owned(), Value::String(text.clone()));
        }
    }
    TextToSpeechRequestData {
        dict_body: Some(body),
        ssml_body: None,
        headers: Some(IndexMap::from([(
            String::from("Content-Type"),
            String::from("application/json"),
        )])),
    }
}
pub fn decode_voxtral_reply(
    request: &TextToSpeechRequestData,
    response: &UpstreamResponse,
) -> Result<BinaryContent, MistralAudioError> {
    let fields = reply_fields(response)?;
    let audio = match fields.get("audio_data") {
        Some(Value::String(encoded)) if !encoded.is_empty() => encoded,
        Some(_) | None => return Err(MistralAudioError::AudioDataMissing),
    };
    let decoded = AUDIO_BASE64
        .decode(audio)
        .map_err(|source| MistralAudioError::AudioNotBase64 { source })?;
    let requested_format = request
        .dict_body
        .as_ref()
        .and_then(|body| body.get("response_format"))
        .and_then(Value::as_str);
    Ok(BinaryContent {
        status: 200,
        headers: audio_headers(&response.headers, requested_format, decoded.len()),
        body: decoded,
    })
}
fn audio_headers(
    headers: &http::HeaderMap,
    requested_format: Option<&str>,
    length: usize,
) -> IndexMap<String, String> {
    let mut rebuilt: IndexMap<String, String> = flatten_headers(headers)
        .into_iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "content-encoding" | "transfer-encoding" | "content-length" | "content-type"
            )
        })
        .collect();
    rebuilt.insert(String::from("content-length"), length.to_string());
    rebuilt.insert(
        String::from("content-type"),
        String::from(mime(requested_format)),
    );
    rebuilt
}

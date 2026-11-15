use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use tracing::warn;
use url::Position;
use url::Url;

use betterlite_models::decisions::Json;

use super::DEFAULT_MUSE_REALTIME_URL;
use super::LANGUAGE_CODES;
use super::MUSE_MODEL;
use super::MuseMode;
use super::MuseProtocolError;
use super::MuseSampleRate;
use super::MuseSessionConfig;
use super::SUPPORTED_LANGUAGES;
use crate::api::api_boundaries::ApiLimits;
use crate::api::transcription_protocol::TranscriptionAudioFormat;
use crate::api::transcription_protocol::TranscriptionLayout;
use crate::api::transcription_protocol::TranscriptionSessionUpdate;
use crate::api::transcription_protocol::parse_transcription_session_update;

#[must_use]
pub fn normalize_model(model: &str) -> String {
    model
        .strip_prefix("meta/")
        .unwrap_or(model)
        .trim()
        .to_owned()
}

pub fn normalize_language(language: &str) -> Result<String, MuseProtocolError> {
    let value = language.trim();
    if value.is_empty() {
        return Err(MuseProtocolError::LanguageEmpty);
    }
    if let Some(name) = SUPPORTED_LANGUAGES
        .iter()
        .find(|name| name.eq_ignore_ascii_case(value))
    {
        return Ok((*name).to_owned());
    }
    let normalized = value.replace('_', "-");
    let primary = normalized
        .split('-')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    match LANGUAGE_CODES
        .iter()
        .find(|(code, _)| *code == primary.as_str())
    {
        Some((_, name)) => Ok((*name).to_owned()),
        None => Err(MuseProtocolError::UnsupportedLanguage),
    }
}

pub fn normalize_access_token(api_key: &SecretString) -> Result<SecretString, MuseProtocolError> {
    let stripped = api_key.expose_secret().trim();
    if stripped.is_empty() {
        return Err(MuseProtocolError::ApiKeyRequired);
    }
    let mut parts = stripped.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or_default();
    if !first.eq_ignore_ascii_case("bearer") {
        return Ok(SecretString::from(format!("Bearer {stripped}")));
    }
    let token = parts.next().map(str::trim).unwrap_or_default();
    if token.is_empty() {
        return Err(MuseProtocolError::BearerTokenMissing);
    }
    Ok(SecretString::from(format!("Bearer {token}")))
}

pub fn build_muse_realtime_url(api_base: Option<&str>) -> Result<String, MuseProtocolError> {
    let Some(api_base) = api_base else {
        return Ok(DEFAULT_MUSE_REALTIME_URL.to_owned());
    };
    let parsed = Url::parse(api_base.trim()).map_err(|_| MuseProtocolError::InvalidApiBase)?;
    let scheme = match parsed.scheme() {
        "https" => "wss",
        other => other,
    };
    let authority = parsed[Position::BeforeUsername..Position::AfterPort];
    let host = parsed.host_str().ok_or(MuseProtocolError::InvalidApiBase)?;
    if scheme != "wss" || authority.contains('@') || parsed.fragment().is_some() {
        return Err(MuseProtocolError::InvalidApiBase);
    }
    let netloc = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    Ok(format!("{scheme}://{netloc}/v1/asr/realtime"))
}

pub fn get_complete_url(api_base: Option<&str>, model: &str) -> Result<String, MuseProtocolError> {
    if normalize_model(model) != MUSE_MODEL {
        return Err(MuseProtocolError::ModelRejected {
            model: model.to_owned(),
        });
    }
    build_muse_realtime_url(api_base)
}

pub fn parse_session_update(
    payload: &str,
    expected_model: &str,
    limits: ApiLimits,
) -> Result<MuseSessionConfig, MuseProtocolError> {
    let update =
        parse_transcription_session_update::<MuseProtocolError>(payload.as_bytes(), limits)?;
    match update.session_type.as_deref() {
        None | Some("transcription" | "realtime") => {}
        Some(_) => return Err(MuseProtocolError::TranscriptionOnly),
    }
    let keys = &update.unsupported_transcription_keys;
    if !keys.is_empty() {
        warn!("Meta realtime: dropping unsupported transcription settings {keys:?}");
    }
    let normalized_model = normalize_model(expected_model);
    if normalized_model != MUSE_MODEL {
        return Err(MuseProtocolError::UnsupportedModel);
    }
    let changes_model = update
        .model
        .as_deref()
        .is_some_and(|model| normalize_model(model) != normalized_model);
    if changes_model {
        return Err(MuseProtocolError::ModelChanged);
    }
    let language_bias = match update.language.as_deref() {
        Some(language) => vec![normalize_language(language)?],
        None => Vec::new(),
    };
    Ok(MuseSessionConfig {
        model: normalized_model,
        mode: parse_mode(&update)?,
        sample_rate: parse_sample_rate(&update)?,
        language_bias,
    })
}

fn parse_mode(update: &TranscriptionSessionUpdate) -> Result<MuseMode, MuseProtocolError> {
    if update.turn_detection_disabled {
        return Ok(MuseMode::PushToTalk);
    }
    match update.turn_detection_type() {
        None => Ok(MuseMode::Endpointing),
        Some(Json::Text(kind)) if kind == "server_vad" => Ok(MuseMode::Endpointing),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        ) => Err(MuseProtocolError::TurnDetectionUnsupported),
    }
}

fn parse_sample_rate(
    update: &TranscriptionSessionUpdate,
) -> Result<MuseSampleRate, MuseProtocolError> {
    let Some(audio_format) = update.audio_format.as_ref() else {
        return Ok(MuseSampleRate::Hz24000);
    };
    match audio_format.layout {
        TranscriptionLayout::Beta => match audio_format.encoding.as_deref() {
            Some("pcm16") => Ok(MuseSampleRate::Hz24000),
            Some(_) | None => Err(MuseProtocolError::BetaEncoding),
        },
        TranscriptionLayout::Ga => ga_sample_rate(audio_format),
    }
}

fn ga_sample_rate(
    audio_format: &TranscriptionAudioFormat,
) -> Result<MuseSampleRate, MuseProtocolError> {
    if !audio_format.is_pcm16() {
        return Err(MuseProtocolError::PcmEncoding);
    }
    let mono = audio_format
        .channels
        .as_ref()
        .is_none_or(|channels| channels.as_i64() == Some(1));
    if !mono {
        return Err(MuseProtocolError::MonoOnly);
    }
    let rate = match audio_format.rate.as_ref() {
        None => 24_000_i64,
        Some(rate) => rate
            .as_i64()
            .ok_or(MuseProtocolError::SampleRateUnsupported)?,
    };
    match rate {
        16_000 => Ok(MuseSampleRate::Hz16000),
        24_000 => Ok(MuseSampleRate::Hz24000),
        _ => Err(MuseProtocolError::SampleRateUnsupported),
    }
}

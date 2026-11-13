use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::multipart::MultipartPart;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api::audio_file_input::ProcessedAudioFile;
use crate::xai_identity::{KeyOrder, get_api_base, get_api_key};

use super::error::XaiSttError;

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["language"];
const EXCLUDED_PARAMS: [&str; 3] = ["model", "OPENAI_TRANSCRIPTION_PARAMS", "extra_body"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum FormScalar {
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum FormValue {
    List(Vec<FormScalar>),
    Scalar(FormScalar),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum FormText {
    Scalar(String),
    Repeated(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct SttForm {
    pub fields: Vec<(String, FormValue)>,
    pub file: MultipartPart,
}

impl FormScalar {
    // Python's str() spelling of a list member, where a bool capitalizes and a float keeps its repr
    #[must_use]
    pub fn member(&self) -> String {
        match self {
            Self::Bool(true) => "True".to_owned(),
            Self::Bool(false) => "False".to_owned(),
            Self::Integer(value) => value.to_string(),
            Self::Float(value) => float_text(*value),
            Self::Text(text) => text.clone(),
            Self::Null => "None".to_owned(),
        }
    }
}

impl FormValue {
    // a scalar bool reaches the form in lowercase, while list members keep Python's str() spelling
    #[must_use]
    pub fn form_text(&self) -> FormText {
        match self {
            Self::Scalar(FormScalar::Bool(flag)) => FormText::Scalar(flag.to_string()),
            Self::Scalar(scalar) => FormText::Scalar(scalar.member()),
            Self::List(items) => FormText::Repeated(items.iter().map(FormScalar::member).collect()),
        }
    }
}

// Python prints a float in fixed notation for magnitudes in [1e-4, 1e16) and in scientific notation outside it
fn float_text(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude != 0.0_f64 && !(1e-4_f64..1e16_f64).contains(&magnitude) {
        let scientific = format!("{value:e}");
        if let Some((mantissa, exponent)) = scientific.split_once('e')
            && let Ok(power) = exponent.parse::<i32>()
        {
            let sign = if power.is_negative() { '-' } else { '+' };
            return format!("{mantissa}e{sign}{:02}", power.unsigned_abs());
        }
        return scientific;
    }
    let fixed = format!("{value}");
    if fixed.contains('.') {
        fixed
    } else {
        format!("{fixed}.0")
    }
}

#[must_use]
pub fn stt_url(api_base: Option<&str>, settings: &Settings) -> String {
    let base = get_api_base(api_base, settings);
    let trimmed = base.trim_end_matches('/');
    let normalized = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    format!("{normalized}/v1/stt")
}

pub fn authorization_headers(
    mut headers: HeaderMap,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<HeaderMap, XaiSttError> {
    let key = get_api_key(api_key, KeyOrder::Chat, settings).ok_or(XaiSttError::MissingApiKey)?;
    let value = HeaderValue::try_from(format!("Bearer {}", key.expose_secret()))
        .map_err(|source| XaiSttError::Authorization { source })?;
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

// LiteLLM's override keeps the caller's optional params and adds only the advertised language
pub fn map_openai_params(
    non_default_params: &[(String, Value)],
    optional_params: &mut Map<String, Value>,
) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// extra_body is flattened first, a direct option of the same name replaces it in place, and excluded names never ship
#[must_use]
pub fn transcription_form(
    model: &str,
    audio: &ProcessedAudioFile,
    extra_body: &IndexMap<String, FormValue>,
    direct: &IndexMap<String, FormValue>,
) -> SttForm {
    let mut flat: IndexMap<String, FormValue> = IndexMap::new();
    for (name, value) in extra_body.iter().chain(direct.iter()) {
        if !EXCLUDED_PARAMS.contains(&name.as_str())
            && !matches!(value, FormValue::Scalar(FormScalar::Null))
        {
            flat.insert(name.clone(), value.clone());
        }
    }
    let mut fields: Vec<(String, FormValue)> = Vec::with_capacity(flat.len().saturating_add(1));
    fields.push((
        "model".to_owned(),
        FormValue::Scalar(FormScalar::Text(model.to_owned())),
    ));
    fields.extend(flat);
    SttForm {
        fields,
        file: MultipartPart {
            name: "file".to_owned(),
            file_name: Some(audio.filename.clone()),
            content_type: Some(audio.content_type.clone()),
            data: audio.file_content.clone(),
        },
    }
}

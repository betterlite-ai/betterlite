use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::TranscriptionError;
use super::mapping::SUPPORTED_OPENAI_PARAMS;

pub const FILE_FIELD: &str = "file";
pub const MODEL_ID_FIELD: &str = "model_id";
// LiteLLM hands these two names to the request layer itself, so they never reach the provider as form fields
const EXCLUDED_PARAMS: [&str; 2] = ["model", "OPENAI_TRANSCRIPTION_PARAMS"];

#[derive(Debug, Clone)]
pub struct TranscriptionForm {
    pub fields: IndexMap<String, String>,
    pub file: MultipartPart,
}

pub fn transform_request(
    model: &str,
    audio: &ProcessedAudioFile,
    optional_params: &Object,
) -> Result<TranscriptionForm, TranscriptionError> {
    let mut fields = IndexMap::new();
    fields.insert(MODEL_ID_FIELD.to_owned(), model.to_owned());
    let supported = optional_params.iter().filter(|(name, value)| {
        !value.is_null() && SUPPORTED_OPENAI_PARAMS.contains(&name.as_str())
    });
    let provider = optional_params.iter().filter(|(name, value)| {
        !value.is_null()
            && !EXCLUDED_PARAMS.contains(&name.as_str())
            && !SUPPORTED_OPENAI_PARAMS.contains(&name.as_str())
    });
    for (name, value) in supported.chain(provider) {
        let text = match (value.as_str(), value.as_bool(), value.as_number()) {
            (Some(text), _, _) => text.to_owned(),
            (_, Some(flag), _) => {
                if flag {
                    "True".to_owned()
                } else {
                    "False".to_owned()
                }
            }
            (_, _, Some(number)) => number_text(number),
            _ => {
                return Err(TranscriptionError::FormValue {
                    field: name.clone(),
                });
            }
        };
        fields.insert(name.clone(), text);
    }
    Ok(TranscriptionForm {
        fields,
        file: MultipartPart {
            name: FILE_FIELD.to_owned(),
            file_name: Some(audio.filename.clone()),
            content_type: Some(audio.content_type.clone()),
            data: audio.file_content.clone(),
        },
    })
}

// the form carries str(value), which keeps an integer literal as written and renders a float with its shortest digits
#[must_use]
pub(super) fn number_text(number: &Number) -> String {
    let literal = number.to_string();
    if literal
        .bytes()
        .any(|byte| matches!(byte, b'.' | b'e' | b'E'))
    {
        return number.as_f64().map_or(literal, float_text);
    }
    if literal == "-0" {
        return "0".to_owned();
    }
    literal
}

fn float_text(value: f64) -> String {
    let shown = format!("{value:?}");
    let Some((mantissa, exponent)) = shown.split_once('e') else {
        return shown;
    };
    let (sign, digits) = if let Some(digits) = exponent.strip_prefix('-') {
        ("-", digits)
    } else if let Some(digits) = exponent.strip_prefix('+') {
        ("+", digits)
    } else {
        ("+", exponent)
    };
    let padded = if digits.len() < 2 {
        format!("0{digits}")
    } else {
        digits.to_owned()
    };
    format!("{mantissa}e{sign}{padded}")
}

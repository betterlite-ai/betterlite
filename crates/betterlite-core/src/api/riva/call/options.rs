use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;

use super::error::{MISSING_ENDPOINT, RivaCallError};
use crate::api::riva::audio::{TARGET_CHANNEL_COUNT, TARGET_SAMPLE_RATE_HZ};
use crate::api::riva::records::error::RecordsError;
use crate::api::riva::records::options::{DEFAULT_ENCODING, map_openai_params};
use crate::api::riva::records::recognition::{RecognitionOptions, RivaRequest, transform_request};

pub const NVCF_FUNCTION_ID: &str = "nvcf_function_id";
pub const USE_SSL: &str = "use_ssl";

// what the call reads off the optional params before any SDK object exists: the B1545 request with the wire format forced to the resampler's output, plus the auth inputs the handler reads raw
#[derive(Clone)]
pub struct Resolved {
    pub request: RivaRequest,
    pub function_id: Option<String>,
    pub use_ssl: bool,
    pub use_ssl_logged: Json,
}

pub fn resolve(params: &Params) -> Result<Resolved, RivaCallError> {
    let optional = params_object(params)?;
    let mapped = map_openai_params(&optional)?;
    let mut request = transform_request(&mapped)?;
    force_wire_format(&mut request.recognition_config);
    let nvcf = function_id(&optional)?;
    let override_value = optional.get(USE_SSL).filter(|value| !value.is_null());
    let override_flag = match override_value {
        Some(value) => {
            let typed = serde_json::from_value::<JsonValue>(value.clone())
                .map_err(|source| RivaCallError::Json { source })?;
            Some(typed)
        }
        None => None,
    };
    let use_ssl = match override_flag {
        Some(JsonValue::Bool(flag)) => flag,
        Some(JsonValue::Number(number)) => number.as_f64().is_some_and(|real| real != 0.0_f64),
        Some(JsonValue::Text(text)) => !text.is_empty(),
        Some(JsonValue::Array(items)) => !items.is_empty(),
        Some(JsonValue::Object(fields)) => !fields.is_empty(),
        Some(JsonValue::Null) | None => nvcf.is_some(),
    };
    let use_ssl_logged = match override_value {
        Some(value) => serde_json::from_value::<Json>(value.clone())
            .map_err(|source| RivaCallError::Json { source })?,
        None => Json::Null,
    };
    Ok(Resolved {
        request,
        function_id: nvcf,
        use_ssl,
        use_ssl_logged,
    })
}

// The resampler fixes the bytes at 16 kHz mono int16, so the config cannot keep a caller's values
fn force_wire_format(recognition: &mut RecognitionOptions) {
    recognition.sample_rate_hertz = TARGET_SAMPLE_RATE_HZ;
    recognition.audio_channel_count = TARGET_CHANNEL_COUNT;
    recognition.encoding = DEFAULT_ENCODING.to_owned();
}

// optional_params crosses into B1545 as the dict a LiteLLM caller passes
fn params_object(params: &Params) -> Result<Object, RivaCallError> {
    let value = serde_json::to_value(params).map_err(|source| RivaCallError::Json { source })?;
    let Some(object) = value.as_object() else {
        return Err(RivaCallError::Params);
    };
    Ok(object.clone())
}

fn function_id(optional: &Object) -> Result<Option<String>, RivaCallError> {
    let Some(value) = optional
        .get(NVCF_FUNCTION_ID)
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(RivaCallError::Records(RecordsError::Text {
            field: NVCF_FUNCTION_ID,
        }));
    };
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(text.to_owned()))
}

// `if not api_base`: the handler refuses a missing or empty endpoint before it touches the audio
pub fn endpoint(api_base: Option<&str>) -> Result<&str, RivaCallError> {
    match api_base {
        Some(value) if !value.is_empty() => Ok(value),
        Some(_empty) | None => Err(RivaCallError::MissingEndpoint {
            message: MISSING_ENDPOINT.to_owned(),
        }),
    }
}

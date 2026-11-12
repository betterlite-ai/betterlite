use betterlite_context::anthropic_catalog_lookup::model_capability;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use serde::Deserialize;

use super::{CatalogSources, DropParams, unsupported_params};

// Claude 4.7+ (Opus 4.7/4.8, Fable 5) rejects top_p, top_k and any temperature but 1.
const SAMPLING_REMOVED_MARKERS: [&str; 9] = [
    "fable", "opus-4-7", "opus_4_7", "opus-4.7", "opus_4.7", "opus-4-8", "opus_4_8", "opus-4.8",
    "opus_4.8",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SamplingParam {
    Temperature,
    TopP,
    TopK,
}

impl SamplingParam {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Temperature => "temperature",
            Self::TopP => "top_p",
            Self::TopK => "top_k",
        }
    }
}

pub struct SamplingRequest<'request> {
    pub model: &'request str,
    pub param: SamplingParam,
    pub value: &'request Json,
    pub drop: DropParams,
}

#[derive(Debug, thiserror::Error)]
pub enum SamplingError {
    #[error(
        "{model} does not support {param}={value}. {hint}To drop unsupported params, set `litellm.drop_params = True`."
    )]
    Unsupported {
        model: String,
        param: &'static str,
        value: String,
        hint: &'static str,
    },
    #[error("the unsupported parameter value could not be rendered: {source}")]
    Value {
        #[source]
        source: serde_json::Error,
    },
}

impl SamplingError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Unsupported { .. } => Some(400),
            Self::Value { .. } => None,
        }
    }

    #[must_use]
    pub fn classify(&self) -> Option<ErrorClass> {
        match self {
            Self::Unsupported { .. } => Some(unsupported_params(self.to_string())),
            Self::Value { .. } => None,
        }
    }
}

#[must_use]
pub fn supports_sampling_params(model: &str, catalogs: &CatalogSources<'_>) -> bool {
    model_capability(
        catalogs.patterns,
        catalogs.runtime,
        catalogs.packaged,
        model,
        "supports_sampling_params",
    )
    .unwrap_or_else(|| {
        let lowered = model.to_lowercase();
        !SAMPLING_REMOVED_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker))
    })
}

// Some goes to the caller's output_key, None omits the field, and the error is LiteLLM's 400.
pub fn apply_sampling_param(
    request: &SamplingRequest<'_>,
    catalogs: &CatalogSources<'_>,
) -> Result<Option<Json>, SamplingError> {
    let temperature_one = request.param == SamplingParam::Temperature && is_one(request.value);
    if supports_sampling_params(request.model, catalogs) || temperature_one {
        return Ok(Some(request.value.clone()));
    }
    if request.drop.global || request.drop.call {
        return Ok(None);
    }
    let hint = match request.param {
        SamplingParam::Temperature => "Only temperature=1 is supported. ",
        SamplingParam::TopP | SamplingParam::TopK => "",
    };
    Err(SamplingError::Unsupported {
        model: request.model.to_owned(),
        param: request.param.key(),
        value: serde_json::to_string(request.value)
            .map_err(|source| SamplingError::Value { source })?,
        hint,
    })
}

// Python's `value == 1` also holds for True and for the float 1.0.
fn is_one(value: &Json) -> bool {
    match value {
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_i64() == Some(1) || number.as_f64() == Some(1.0),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
    }
}

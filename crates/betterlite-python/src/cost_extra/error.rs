use std::fmt::Display;

use betterlite_core::cost::batch::error::BatchCostError;
use betterlite_core::cost::completion::error::ChargeInputError;
use betterlite_core::cost::realtime::error::RealtimeCostError;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::model_metadata_resolution::error::model_not_mapped_message;
use betterlite_models::ocr_pricing::OcrCostError;
use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use betterlite_models::resolution::ResolutionError;
use pyo3::PyErr;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use serde::Serialize;

use crate::cost_extra::convert::ConvertError;
use crate::cost_extra::settings::SettingsError;

const BAD_REQUEST: &str = "BadRequestError";
const MODEL_NOT_MAPPED: &str = "ModelNotMappedError";

// The two failures that mix a core error with the clock the engine read before it.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RerankFailure {
    #[error("resolving the provider for the rerank model failed: {source}")]
    Resolution {
        #[from]
        source: ResolutionError,
    },
    #[error("a rerank price does not parse as an exact decimal: {0}")]
    Rate(#[from] DecimalLexemeError),
    #[error("the rerank cost arithmetic overflowed")]
    Overflow,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RealtimeFailure {
    #[error(transparent)]
    Cost(#[from] RealtimeCostError),
    #[error("the realtime clock could not be read: {0}")]
    Clock(#[from] ChargeInputError),
}

// The JSON shape betterlite._completion.errors reads: one class name and its message.
#[derive(Debug, Serialize)]
pub(crate) struct CostExtraPayload {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) message: String,
}

// ast-grep-ignore: no-one-line-functions
pub(crate) fn payload(message: String) -> CostExtraPayload {
    CostExtraPayload {
        kind: "Exception",
        message,
    }
}

// The class completion's family registered, so Python can raise it through raise_native_error.
pub(crate) fn native_error(py: Python<'_>, payload: CostExtraPayload) -> PyErr {
    let text = match serde_json::to_string(&payload) {
        Ok(text) => text,
        Err(_encoding) => return PyRuntimeError::new_err(payload.message),
    };
    let class = match py.import("betterlite._native") {
        Ok(module) => module.getattr("NativeError"),
        Err(failure) => return failure,
    };
    match class.and_then(|found| found.call1((text,))) {
        Ok(value) => PyErr::from_value(value),
        Err(failure) => failure,
    }
}

// LiteLLM's bare Exception, with betterlite's own message, for every failure it does not name a class for.
pub(crate) fn generic_error(py: Python<'_>, error: &dyn Display) -> PyErr {
    native_error(py, payload(error.to_string()))
}

// LiteLLM's own exception class, built with the message and, for BadRequestError, the model.
fn exception_error(py: Python<'_>, name: &str, message: String, model: Option<&str>) -> PyErr {
    let class = match py
        .import("betterlite.exceptions")
        .and_then(|module| module.getattr(name))
    {
        Ok(found) => found,
        Err(failure) => return failure,
    };
    let called = match model {
        Some(missing) => class.call1((message.as_str(), missing, "")),
        None => class.call1((message,)),
    };
    match called {
        Ok(value) => PyErr::from_value(value),
        Err(failure) => failure,
    }
}

// get_llm_provider's text when no provider can be told from the model or its name.
fn provider_missing_message(model: &str) -> String {
    format!(
        "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed \
model={model}\n Pass model as E.g. For 'Huggingface' inference endpoints pass in \
`completion(model='huggingface/starcoder',..)` Learn more: https://docs.litellm.ai/docs/providers"
    )
}

// get_llm_provider reports every provider failure as a BadRequestError, so no caller sees the cause shape.
pub(crate) fn resolution_error(py: Python<'_>, error: &ResolutionError, model: &str) -> PyErr {
    match error {
        ResolutionError::MissingProvider { .. } => exception_error(
            py,
            BAD_REQUEST,
            provider_missing_message(model),
            Some(model),
        ),
        ResolutionError::EndpointWithoutProvider { .. }
        | ResolutionError::DefaultsA { .. }
        | ResolutionError::DefaultsB { .. } => {
            exception_error(py, BAD_REQUEST, error.to_string(), Some(model))
        }
    }
}

// ocr_cost raises ValueError for a missing response, usage or page count; every OCR failure lands there.
// ast-grep-ignore: no-one-line-functions
pub(crate) fn ocr_error(error: &OcrCostError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

// The image calculator's only ValueError is a size that does not split into two integers.
pub(crate) fn image_error(py: Python<'_>, error: &ImageCostError) -> PyErr {
    match error {
        ImageCostError::MalformedSize { .. } => PyValueError::new_err(error.to_string()),
        ImageCostError::UnknownModel { .. }
        | ImageCostError::NotImageResponse
        | ImageCostError::NoPrice { .. }
        | ImageCostError::WrongFieldType { .. }
        | ImageCostError::DimensionOverflow
        | ImageCostError::CounterOverflow
        | ImageCostError::Overflow
        | ImageCostError::Rate(_)
        | ImageCostError::Tier(_)
        | ImageCostError::TokenCost(_) => generic_error(py, error),
    }
}

// The video calculator raises a bare Exception naming the model it could not price.
pub(crate) fn video_error(py: Python<'_>, error: &DurationCostError) -> PyErr {
    match error {
        DurationCostError::UnknownModel { model } => generic_error(
            py,
            &format!("Model not found in cost map for model={model}"),
        ),
        DurationCostError::NoCharacterOrTokenPricing { .. }
        | DurationCostError::Overflow
        | DurationCostError::Price(_)
        | DurationCostError::TokenCost(_) => generic_error(py, error),
    }
}

// openai's cost_per_second reads the model through get_model_info, whose unmapped class is ModelNotMappedError.
pub(crate) fn transcription_error(
    py: Python<'_>,
    error: &DurationCostError,
    model: &str,
    provider: Option<&str>,
) -> PyErr {
    match error {
        DurationCostError::UnknownModel { .. } => exception_error(
            py,
            MODEL_NOT_MAPPED,
            model_not_mapped_message(model, provider),
            None,
        ),
        DurationCostError::NoCharacterOrTokenPricing { .. }
        | DurationCostError::Overflow
        | DurationCostError::Price(_)
        | DurationCostError::TokenCost(_) => generic_error(py, error),
    }
}

pub(crate) fn batch_error(py: Python<'_>, error: &BatchCostError, model: &str) -> PyErr {
    match error {
        BatchCostError::Resolution { source } => resolution_error(py, source, model),
        BatchCostError::Rate(source) => generic_error(py, source),
        BatchCostError::Modality(source) => generic_error(py, source),
        BatchCostError::Overflow => generic_error(py, error),
    }
}

pub(crate) fn rerank_error(py: Python<'_>, error: &RerankFailure, model: &str) -> PyErr {
    match error {
        RerankFailure::Resolution { source } => resolution_error(py, source, model),
        RerankFailure::Rate(source) => generic_error(py, source),
        RerankFailure::Overflow => generic_error(py, error),
    }
}

pub(crate) fn realtime_error(py: Python<'_>, error: &RealtimeFailure) -> PyErr {
    match error {
        RealtimeFailure::Cost(source) => generic_error(py, source),
        RealtimeFailure::Clock(source) => generic_error(py, source),
    }
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        match &error {
            ConvertError::Json { .. } => PyValueError::new_err(error.to_string()),
            ConvertError::Encode { .. } => PyRuntimeError::new_err(error.to_string()),
        }
    }
}

// The engine could not be built from the settings text Python passed.
impl From<SettingsError> for PyErr {
    fn from(error: SettingsError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

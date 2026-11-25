use betterlite_core::cost::batch::error::BatchCostError;
use betterlite_core::cost::completion::error::ChargeInputError;
use betterlite_core::cost::realtime::error::RealtimeCostError;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::model_metadata_resolution::error::model_not_mapped_message;
use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use betterlite_models::resolution::ResolutionError;
use pyo3::{PyErr, exceptions::PyValueError, prelude::*};

use crate::cost::error::{native_error, payload};
use crate::cost_extra::convert::TranscriptionRequest;

mod tests;

const BAD_REQUEST: &str = "BadRequestError";
const MODEL_NOT_MAPPED: &str = "ModelNotMappedError";

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
    #[error("'{provider}' is not a valid LlmProviders")]
    UnknownProvider { provider: String },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RealtimeFailure {
    #[error(transparent)]
    Cost(#[from] RealtimeCostError),
    #[error("the realtime clock could not be read: {0}")]
    Clock(#[from] ChargeInputError),
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
        "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model={model}\n Pass model as E.g. For 'Huggingface' inference endpoints pass in `completion(model='huggingface/starcoder',..)` Learn more: https://docs.litellm.ai/docs/providers"
    )
}

// get_llm_provider reports every provider failure as a BadRequestError, so no caller sees the cause shape.
pub(crate) fn resolution_error(py: Python<'_>, error: &ResolutionError, model: &str) -> PyErr {
    let message = match error {
        ResolutionError::MissingProvider { .. } => provider_missing_message(model),
        ResolutionError::EndpointWithoutProvider { .. }
        | ResolutionError::DefaultsA { .. }
        | ResolutionError::DefaultsB { .. } => error.to_string(),
    };
    exception_error(py, BAD_REQUEST, message, Some(model))
}

pub(crate) fn image_error(py: Python<'_>, error: &ImageCostError) -> PyErr {
    if let ImageCostError::MalformedSize { .. } = error {
        return PyValueError::new_err(error.to_string());
    }
    native_error(py, payload(error.to_string()))
}

// The video calculator raises a bare Exception naming the model it could not price.
pub(crate) fn video_error(py: Python<'_>, error: &DurationCostError) -> PyErr {
    let DurationCostError::UnknownModel { model } = error else {
        return native_error(py, payload(error.to_string()));
    };
    native_error(
        py,
        payload(format!("Model not found in cost map for model={model}")),
    )
}

// openai's cost_per_second reads the model through get_model_info, whose unmapped class is ModelNotMappedError.
pub(crate) fn transcription_error(
    py: Python<'_>,
    error: &DurationCostError,
    request: &TranscriptionRequest,
) -> PyErr {
    if let DurationCostError::UnknownModel { .. } = error {
        let provider = request.custom_llm_provider.as_deref();
        let message = model_not_mapped_message(&request.model, provider);
        return exception_error(py, MODEL_NOT_MAPPED, message, None);
    }
    native_error(py, payload(error.to_string()))
}

// The batch calculator's rate and modality failures keep the inner error's own text.
pub(crate) fn batch_error(py: Python<'_>, error: &BatchCostError, model: &str) -> PyErr {
    if let BatchCostError::Resolution { source } = error {
        return resolution_error(py, source, model);
    }
    native_error(py, payload(error.to_string()))
}

pub(crate) fn rerank_error(py: Python<'_>, error: &RerankFailure, model: &str) -> PyErr {
    match error {
        RerankFailure::Resolution { source } => resolution_error(py, source, model),
        RerankFailure::Rate(source) => native_error(py, payload(source.to_string())),
        RerankFailure::Overflow => native_error(py, payload(error.to_string())),
        RerankFailure::UnknownProvider { .. } => PyValueError::new_err(error.to_string()),
    }
}

pub(crate) fn realtime_error(py: Python<'_>, error: &RealtimeFailure) -> PyErr {
    match error {
        RealtimeFailure::Cost(source) => native_error(py, payload(source.to_string())),
        RealtimeFailure::Clock(source) => native_error(py, payload(source.to_string())),
    }
}

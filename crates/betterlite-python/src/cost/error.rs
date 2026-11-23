use betterlite_core::cost::call_price::error::CallPriceError;
use betterlite_core::cost::completion::error::{ChargeInputError, CompletionCostError};
use betterlite_core::cost::completion::usage::UsageConversionError;
use betterlite_core::cost::prepare::error::PrepareError;
use betterlite_core::cost::usage::BillingUsageError;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::model_metadata_resolution::error::model_not_mapped_message;
use betterlite_models::resolution::ResolutionError;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_provider::pricing::image::error::ImagePolicyError;
use pyo3::PyErr;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use serde::Serialize;

use crate::cost::billing::CompletionRequest;
use crate::cost::convert::ConvertError;
use crate::cost::settings::SettingsError;

mod tests;

// What one cost call can fail with, gathered from the B068 stages it runs.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EngineError {
    #[error(transparent)]
    Usage(#[from] BillingUsageError),
    #[error(transparent)]
    Conversion(#[from] UsageConversionError),
    #[error(transparent)]
    Input(#[from] ChargeInputError),
    #[error(transparent)]
    Price(#[from] CallPriceError),
    #[error(transparent)]
    Completion(#[from] CompletionCostError<ImagePolicyError>),
}

// The JSON shape betterlite._completion.errors reads: one class name and its message.
#[derive(Debug, Serialize)]
pub(crate) struct CostErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) message: String,
}

// The class completion's family registered, so Python can raise it through raise_native_error.
pub(crate) fn native_error(py: Python<'_>, payload: CostErrorPayload) -> PyErr {
    let text = match serde_json::to_string(&payload) {
        Ok(text) => text,
        Err(_encoding) => return PyRuntimeError::new_err(payload.message),
    };
    let class = match py.import("betterlite._native") {
        Ok(module) => module.getattr("NativeError"),
        Err(failure) => return failure,
    };
    match class.and_then(|class| class.call1((text,))) {
        Ok(value) => PyErr::from_value(value),
        Err(failure) => failure,
    }
}

pub(crate) fn payload(message: String) -> CostErrorPayload {
    CostErrorPayload {
        kind: "Exception",
        message,
    }
}

// completion_cost's and response_cost's failures: the response's model can be the only one named.
pub(crate) fn charge_error(
    py: Python<'_>,
    error: &EngineError,
    request: &CompletionRequest,
) -> PyErr {
    if let EngineError::Completion(CompletionCostError::Prepare(PrepareError::ModelMissing)) = error
    {
        return PyValueError::new_err(MODEL_MISSING);
    }
    if let EngineError::Completion(CompletionCostError::Prepare(PrepareError::Counting(_))) = error
    {
        return native_error(py, payload(UNCOUNTED.to_owned()));
    }
    let model = match request.model.as_deref() {
        Some(found) => found,
        None => request
            .response
            .as_ref()
            .and_then(|response| response.model.as_deref())
            .unwrap_or_default(),
    };
    priced_error(py, error, model, request.custom_llm_provider.as_deref())
}

const MODEL_MISSING: &str = "Model is None and does not exist in passed completion_response. Passed completion_response=None, model=None";

// LiteLLM tokenizes the request when a response carries no usage; this family prices usage only.
const UNCOUNTED: &str = "betterlite's Python cost helpers price a response's usage, and this call \
passed a response without usage or request text whose tokens would have to be counted. Pass a \
response with usage, or the prompt and completion with a custom token count.";

// LiteLLM reports every model it cannot price with this one sentence, whichever lookup stage failed.
pub(crate) fn priced_error(
    py: Python<'_>,
    error: &EngineError,
    model: &str,
    provider: Option<&str>,
) -> PyErr {
    let failure = price_failure(error);
    if let Some(
        CallPriceError::Generic(TokenCostError::UnknownModel)
        | CallPriceError::Duration(DurationCostError::UnknownModel { .. }),
    ) = failure
    {
        let message = model_not_mapped_message(model, provider);
        return exception_error(py, "ModelNotMappedError", message, None);
    }
    if let Some(CallPriceError::Resolution {
        source: ResolutionError::MissingProvider { model: missing },
    }) = failure
    {
        return exception_error(
            py,
            "BadRequestError",
            provider_missing_message(missing),
            Some(missing.as_str()),
        );
    }
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

fn price_failure(error: &EngineError) -> Option<&CallPriceError> {
    match error {
        EngineError::Price(found) => Some(found),
        EngineError::Completion(CompletionCostError::Price(found)) => Some(found),
        EngineError::Completion(_)
        | EngineError::Usage(_)
        | EngineError::Input(_)
        | EngineError::Conversion(_) => None,
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

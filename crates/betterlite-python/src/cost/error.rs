use betterlite_core::cost::call_price::error::CallPriceError;
use betterlite_core::cost::completion::error::{ChargeInputError, CompletionCostError};
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

use crate::cost::billing::{CallRequest, CompletionRequest};
use crate::cost::convert::ConvertError;
use crate::cost::settings::SettingsError;

mod tests;

// What one cost call can fail with, gathered from the B068 stages it runs.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EngineError {
    #[error(transparent)]
    Usage(#[from] BillingUsageError),
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
fn native_error(py: Python<'_>, payload: CostErrorPayload) -> PyErr {
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

fn payload(message: String) -> CostErrorPayload {
    CostErrorPayload {
        kind: "Exception",
        message,
    }
}

fn exception_class<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("betterlite.exceptions")?.getattr(name)
}

// LiteLLM's own class for a model it has no price for, raised with its own sentence.
fn not_mapped_error(py: Python<'_>, message: String) -> PyErr {
    match exception_class(py, "ModelNotMappedError").and_then(|class| class.call1((message,))) {
        Ok(value) => PyErr::from_value(value),
        Err(failure) => failure,
    }
}

// LiteLLM raises this one with the model, an empty provider and get_llm_provider's text.
fn provider_missing_error(py: Python<'_>, message: String, model: &str) -> PyErr {
    match exception_class(py, "BadRequestError")
        .and_then(|class| class.call1((message.as_str(), model, "")))
    {
        Ok(value) => PyErr::from_value(value),
        Err(failure) => failure,
    }
}

// cost_per_token's failures: LiteLLM's class for each model it cannot price.
pub(crate) fn call_error(py: Python<'_>, error: &EngineError, request: &CallRequest) -> PyErr {
    let model = request.model.as_deref().unwrap_or_default();
    priced_error(py, error, model, request.custom_llm_provider.as_deref())
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

// get_usage_object's failures: LiteLLM's conversion raises one plain Exception.
pub(crate) fn usage_error(py: Python<'_>, error: &EngineError) -> PyErr {
    native_error(py, payload(error.to_string()))
}

const MODEL_MISSING: &str = "Model is None and does not exist in passed completion_response. Passed completion_response=None, model=None";

// LiteLLM tokenizes the request when a response carries no usage; this family prices usage only.
const UNCOUNTED: &str = "betterlite's Python cost helpers price a response's usage, and this call \
passed a response without usage or request text whose tokens would have to be counted. Pass a \
response with usage, or the prompt and completion with a custom token count.";

// LiteLLM reports every model it cannot price with this one sentence, whichever lookup stage failed.
fn priced_error(py: Python<'_>, error: &EngineError, model: &str, provider: Option<&str>) -> PyErr {
    match price_failure(error) {
        Some(CallPriceError::Generic(TokenCostError::UnknownModel))
        | Some(CallPriceError::Duration(DurationCostError::UnknownModel { .. })) => {
            not_mapped_error(py, model_not_mapped_message(model, provider))
        }
        Some(CallPriceError::Resolution {
            source: ResolutionError::MissingProvider { model: missing },
        }) => provider_missing_error(py, provider_missing_message(missing), missing),
        _ => native_error(py, payload(error.to_string())),
    }
}

fn price_failure(error: &EngineError) -> Option<&CallPriceError> {
    match error {
        EngineError::Price(found) => Some(found),
        EngineError::Completion(CompletionCostError::Price(found)) => Some(found),
        EngineError::Completion(_) | EngineError::Usage(_) | EngineError::Input(_) => None,
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

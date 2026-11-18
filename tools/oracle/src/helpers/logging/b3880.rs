#[path = "b3880/catalog.rs"]
mod catalog;
#[path = "b3880/corpus/tests.rs"]
mod corpus;
#[path = "b3880/exception.rs"]
mod exception;
#[path = "b3880/handlers.rs"]
mod handlers;
#[path = "b3880/inputs.rs"]
mod inputs;
#[path = "b3880/provider_from_exception/tests.rs"]
mod provider_from_exception;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

pub(crate) struct Op;

// the label helper a case asks for, one variant per LiteLLM function this task ports
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ExtractStatusCode(inputs::StatusCodeInput),
    IsInvalidApiKeyRequest(inputs::InvalidKeyInput),
    ShouldSkipMetricsForInvalidKey(inputs::StatusCodeInput),
    ExtractDeploymentFailureLabelValues(inputs::DeploymentLabelsInput),
    GetExceptionClassName(inputs::ExceptionCaseInput),
    BoundedRequestedModelLabel(inputs::BoundedModelInput),
    ExtractRateLimitLabels(inputs::RateLimitInput),
    ExtractApiProviderFromRequestData(inputs::ProviderRequestInput),
    ExtractApiProviderFromException(inputs::ExceptionCaseInput),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ExtractStatusCode(case) => handlers::status_code(case),
            Input::IsInvalidApiKeyRequest(case) => handlers::invalid_key(case),
            Input::ShouldSkipMetricsForInvalidKey(case) => handlers::skip_invalid_key(case),
            Input::ExtractDeploymentFailureLabelValues(case) => handlers::deployment_labels(case),
            Input::GetExceptionClassName(case) => handlers::class_name(case),
            Input::BoundedRequestedModelLabel(case) => handlers::requested_model(case),
            Input::ExtractRateLimitLabels(case) => handlers::rate_limit(case),
            Input::ExtractApiProviderFromRequestData(case) => handlers::request_provider(case),
            Input::ExtractApiProviderFromException(case) => handlers::exception_provider(case),
        }
    }
}

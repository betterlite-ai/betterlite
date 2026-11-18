#[path = "b3880/catalog.rs"]
mod catalog;
#[path = "b3880/corpus/tests.rs"]
mod corpus;
#[path = "b3880/exception.rs"]
mod exception;
#[path = "b3880/inputs.rs"]
mod inputs;
#[path = "b3880/provider_from_exception/tests.rs"]
mod provider_from_exception;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_logging::prometheus::failure::deployment_labels::{
    DeploymentFailureLabels, DeploymentFailureSources, PayloadMetadata, deployment_failure_labels,
};
use betterlite_logging::prometheus::failure::exception_name::exception_class_name;
use betterlite_logging::prometheus::failure::invalid_key::{
    is_invalid_api_key_request,
    // ast-grep-ignore: no-long-names
    should_skip_metrics_for_invalid_key as skip_invalid_key_metrics,
};
use betterlite_logging::prometheus::failure::model_cardinality::bounded_requested_model_label;
use betterlite_logging::prometheus::failure::provider::{
    ProviderRequestData, api_provider_from_exception, api_provider_from_request_data,
};
use betterlite_logging::prometheus::failure::rate_limit::extract_rate_limit_labels;
use betterlite_logging::prometheus::failure::status_code::extract_status_code;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ExtractStatusCode(inputs::StatusCodeInput),
    IsInvalidApiKeyRequest(inputs::InvalidKeyInput),
    ShouldSkipMetricsForInvalidKey(inputs::StatusCodeInput),
    DeploymentFailureLabels(inputs::DeploymentLabelsInput),
    GetExceptionClassName(inputs::ExceptionCaseInput),
    BoundedRequestedModelLabel(inputs::BoundedModelInput),
    ExtractRateLimitLabels(inputs::RateLimitInput),
    ApiProviderFromRequestData(inputs::ProviderRequestInput),
    ApiProviderFromException(inputs::ExceptionCaseInput),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ExtractStatusCode(case) => status_code_answer(case),
            Input::IsInvalidApiKeyRequest(case) => invalid_key_answer(case),
            Input::ShouldSkipMetricsForInvalidKey(case) => {
                let (exception, kwargs_exception) =
                    exception::label_pair(case.exception.as_ref(), case.kwargs_exception.as_ref());
                serde_json::to_value(skip_invalid_key_metrics(
                    case.enum_status_code.as_deref(),
                    exception.as_ref(),
                    kwargs_exception.as_ref(),
                ))
                .or_value_error()
            }
            Input::DeploymentFailureLabels(case) => {
                serde_json::to_value(deployment_answer(case)).or_value_error()
            }
            Input::GetExceptionClassName(case) => serde_json::to_value(exception_class_name(
                &exception::label_exception(&case.exception),
            ))
            .or_value_error(),
            Input::BoundedRequestedModelLabel(case) => {
                serde_json::to_value(model_answer(case)).or_value_error()
            }
            Input::ExtractRateLimitLabels(case) => {
                let exception = case.exception.as_ref().map(exception::label_exception);
                serde_json::to_value(extract_rate_limit_labels(exception.as_ref())).or_value_error()
            }
            Input::ApiProviderFromRequestData(case) => request_provider_answer(case),
            Input::ApiProviderFromException(case) => exception_provider_answer(case),
        }
    }
}

fn status_code_answer(case: &inputs::StatusCodeInput) -> HelperResult {
    let (exception, kwargs) =
        exception::label_pair(case.exception.as_ref(), case.kwargs_exception.as_ref());
    serde_json::to_value(extract_status_code(
        case.enum_status_code.as_deref(),
        exception.as_ref(),
        kwargs.as_ref(),
    ))
    .or_value_error()
}

fn invalid_key_answer(case: &inputs::InvalidKeyInput) -> HelperResult {
    let exception = case.exception.as_ref().map(exception::label_exception);
    serde_json::to_value(is_invalid_api_key_request(
        case.status_code,
        exception.as_ref(),
    ))
    .or_value_error()
}

fn deployment_answer(case: &inputs::DeploymentLabelsInput) -> DeploymentFailureLabels {
    let facts = case
        .user_api_key_auth
        .as_ref()
        .map(inputs::AuthInput::facts);
    let sources = DeploymentFailureSources {
        payload_metadata: case.payload_metadata.as_ref().map(PayloadMetadata::Mapping),
        litellm_params_metadata: case.litellm_params_metadata.as_ref(),
        user_api_key_auth: facts.as_ref(),
    };
    deployment_failure_labels(&sources)
}

fn model_answer(case: &inputs::BoundedModelInput) -> Option<String> {
    let catalog = case.catalog.as_ref().map(catalog::boxed);
    bounded_requested_model_label(
        case.requested_model.as_ref(),
        case.router_originated,
        catalog.as_deref(),
    )
}

fn request_provider_answer(case: &inputs::ProviderRequestInput) -> HelperResult {
    let request = ProviderRequestData {
        litellm_params: case.litellm_params.as_ref(),
        standard_logging: case.standard_logging.as_ref(),
        model: case.model.as_deref(),
    };
    let model_catalog =
        ModelCatalog::parse(catalog::PROVIDER_CATALOG.as_bytes(), UnknownFields::Reject)
            .or_config_error()?;
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &model_catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    serde_json::to_value(api_provider_from_request_data(&request, &context)).or_value_error()
}

fn exception_provider_answer(case: &inputs::ExceptionCaseInput) -> HelperResult {
    serde_json::to_value(api_provider_from_exception(&exception::label_exception(
        &case.exception,
    )))
    .or_value_error()
}

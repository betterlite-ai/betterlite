use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus::failure::deployment_labels::{
    DeploymentFailureSources, PayloadMetadata, UserApiKeyFacts,
    extract_deployment_failure_label_values,
};
use betterlite_logging::prometheus::failure::exception_name::exception_class_name;
use betterlite_logging::prometheus::failure::invalid_key::{
    is_invalid_api_key_request, should_skip_metrics_for_invalid_key,
};
use betterlite_logging::prometheus::failure::model_cardinality::bounded_requested_model_label;
use betterlite_logging::prometheus::failure::provider::{
    ProviderRequestData, extract_api_provider_from_exception,
    extract_api_provider_from_request_data,
};
use betterlite_logging::prometheus::failure::rate_limit::extract_rate_limit_labels;
use betterlite_logging::prometheus::failure::status_code::extract_status_code;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::catalog::{CaseCatalog, ProviderFixture};
use super::exception::label_exception;
use super::inputs::{
    BoundedModelInput, DeploymentLabelsInput, ExceptionCaseInput, InvalidKeyInput,
    ProviderRequestInput, RateLimitInput, StatusCodeInput,
};

pub(crate) fn status_code(case: &StatusCodeInput) -> HelperResult {
    let exception = case.exception.as_ref().map(label_exception);
    let kwargs_exception = case.kwargs_exception.as_ref().map(label_exception);
    serde_json::to_value(extract_status_code(
        case.enum_status_code.as_deref(),
        exception.as_ref(),
        kwargs_exception.as_ref(),
    ))
    .or_value_error()
}

pub(crate) fn invalid_key(case: &InvalidKeyInput) -> HelperResult {
    let exception = case.exception.as_ref().map(label_exception);
    serde_json::to_value(is_invalid_api_key_request(
        case.status_code,
        exception.as_ref(),
    ))
    .or_value_error()
}

pub(crate) fn skip_invalid_key(case: &StatusCodeInput) -> HelperResult {
    let exception = case.exception.as_ref().map(label_exception);
    let kwargs_exception = case.kwargs_exception.as_ref().map(label_exception);
    serde_json::to_value(should_skip_metrics_for_invalid_key(
        case.enum_status_code.as_deref(),
        exception.as_ref(),
        kwargs_exception.as_ref(),
    ))
    .or_value_error()
}

pub(crate) fn deployment_labels(case: &DeploymentLabelsInput) -> HelperResult {
    let payload = case.payload_metadata.as_ref().map(PayloadMetadata::Mapping);
    let auth = case.user_api_key_auth.as_ref().map(|auth| UserApiKeyFacts {
        key_alias: auth.key_alias.clone(),
        user_email: auth.user_email.clone(),
        team_id: auth.team_id.clone(),
        team_alias: auth.team_alias.clone(),
        hashed_key: auth.api_key.clone(),
        key_hash: None,
    });
    let sources = DeploymentFailureSources {
        payload_metadata: payload,
        litellm_params_metadata: case.litellm_params_metadata.as_ref(),
        user_api_key_auth: auth.as_ref(),
    };
    serde_json::to_value(extract_deployment_failure_label_values(&sources)).or_value_error()
}

pub(crate) fn class_name(case: &ExceptionCaseInput) -> HelperResult {
    serde_json::to_value(exception_class_name(&label_exception(&case.exception))).or_value_error()
}

pub(crate) fn requested_model(case: &BoundedModelInput) -> HelperResult {
    let catalog = case.catalog.as_ref().map(CaseCatalog::new);
    serde_json::to_value(bounded_requested_model_label(
        case.requested_model.as_ref(),
        case.router_originated,
        catalog
            .as_ref()
            .map(|catalog| catalog as &dyn RequestedModelCatalog),
    ))
    .or_value_error()
}

pub(crate) fn rate_limit(case: &RateLimitInput) -> HelperResult {
    let exception = case.exception.as_ref().map(label_exception);
    serde_json::to_value(extract_rate_limit_labels(exception.as_ref())).or_value_error()
}

pub(crate) fn request_provider(case: &ProviderRequestInput) -> HelperResult {
    let fixture = ProviderFixture::new()?;
    let request = ProviderRequestData {
        litellm_params: case.litellm_params.as_ref(),
        standard_logging: case.standard_logging.as_ref(),
        model: case.model.as_deref(),
    };
    serde_json::to_value(extract_api_provider_from_request_data(
        &request,
        &fixture.context(),
    ))
    .or_value_error()
}

pub(crate) fn exception_provider(case: &ExceptionCaseInput) -> HelperResult {
    serde_json::to_value(extract_api_provider_from_exception(&label_exception(
        &case.exception,
    )))
    .or_value_error()
}

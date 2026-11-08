use betterlite_error::normalize::ErrorFacts;
use betterlite_error::normalize::Normalizer;
use betterlite_error::rate_limit::RateLimitDimension;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::fields::payload::records::ErrorInformation;
use crate::logging_metadata::decode::{DecodeError, decode};
use crate::logging_metadata::exception::{
    ExceptionFacts, budget_fields, exception_code, provider_request_id, semantic_class,
    validated_category, validated_dimension, validated_type,
};
use crate::logging_metadata::params::MetadataMap;

const MAXIMUM_TRACEBACK_LINES_TO_LOG: usize = 100;

const CLIENT_DISCONNECTED_MESSAGE: &str = "Client disconnected the request";

#[derive(Clone)]
pub struct ExceptionLogging<'input> {
    pub exception: Option<&'input ExceptionFacts>,
    pub log_client_error_tracebacks: bool,
    pub expected_client_error: bool,
    pub redactor: Option<&'input Redactor>,
    pub normalizer: &'input Normalizer,
}

#[derive(Clone)]
pub struct PayloadErrorRequest<'input> {
    pub context: &'input ExceptionLogging<'input>,
    pub metadata: &'input MetadataMap,
    pub error_str: Option<String>,
}

#[derive(Debug, Error)]
pub enum ErrorInformationError {
    #[error("metadata.client_disconnected must be a boolean")]
    ClientDisconnectedType,
    #[error(transparent)]
    StoredRecord(#[from] DecodeError),
}

#[must_use]
pub fn get_error_information(
    context: &ExceptionLogging<'_>,
    traceback: Option<&str>,
) -> ErrorInformation {
    let Some(exception) = context.exception else {
        return empty_information(context, traceback);
    };
    let error_code = exception_code(exception);
    let message = exception.message.clone().unwrap_or_default();
    let dimension = validated_dimension(exception.rate_limit_type.as_deref());
    let (entity_type, entity_id, limit, spend) = budget_fields(exception);
    ErrorInformation {
        error_code: Some(Some(error_code.clone())),
        error_class: Some(Some(exception.class_name.clone())),
        llm_provider: Some(Some(exception.llm_provider.clone().unwrap_or_default())),
        traceback: Some(Some(redact(
            context.redactor,
            &traceback_text(context, exception, traceback),
        ))),
        error_message: Some(Some(redact(context.redactor, &message))),
        error_provider_request_id: Some(provider_request_id(exception)),
        error_rate_limit_category: Some(validated_category(exception.category.as_deref())),
        error_rate_limit_type: Some(validated_type(exception.rate_limit_type.as_deref())),
        error_budget_entity_type: Some(entity_type),
        error_budget_entity_id: Some(entity_id),
        normalized_error: Some(normalized_error(
            &message,
            &error_code,
            exception,
            dimension,
            context.normalizer,
        )),
        error_budget_limit: Some(limit),
        error_budget_spend: Some(spend),
    }
}

pub fn payload_error_information(
    request: &PayloadErrorRequest<'_>,
) -> Result<(ErrorInformation, Option<String>), ErrorInformationError> {
    let information = get_error_information(request.context, None);
    if !client_disconnected(request.metadata)? {
        return Ok((information, request.error_str.clone()));
    }
    let disconnected = client_disconnect_information(request.metadata)?;
    let error_str = request
        .error_str
        .clone()
        .or_else(|| Some(CLIENT_DISCONNECTED_MESSAGE.to_owned()));
    Ok((disconnected, error_str))
}

fn empty_information(context: &ExceptionLogging<'_>, traceback: Option<&str>) -> ErrorInformation {
    ErrorInformation {
        error_code: Some(Some(String::new())),
        error_class: Some(Some(String::new())),
        llm_provider: Some(Some(String::new())),
        traceback: Some(Some(redact(
            context.redactor,
            traceback.unwrap_or_default(),
        ))),
        error_message: Some(Some(redact(context.redactor, ""))),
        error_provider_request_id: Some(None),
        error_rate_limit_category: Some(None),
        error_rate_limit_type: Some(None),
        error_budget_entity_type: Some(None),
        error_budget_entity_id: Some(None),
        normalized_error: Some(None),
        error_budget_limit: Some(None),
        error_budget_spend: Some(None),
    }
}

fn traceback_text(
    context: &ExceptionLogging<'_>,
    exception: &ExceptionFacts,
    traceback: Option<&str>,
) -> String {
    let mut text = traceback.unwrap_or_default().to_owned();
    if !context.log_client_error_tracebacks && context.expected_client_error {
        return text;
    }
    for line in exception
        .traceback_lines
        .iter()
        .take(MAXIMUM_TRACEBACK_LINES_TO_LOG)
    {
        text.push_str(line);
    }
    text
}

fn normalized_error(
    message: &str,
    error_code: &str,
    exception: &ExceptionFacts,
    dimension: Option<RateLimitDimension>,
    normalizer: &Normalizer,
) -> Option<String> {
    let facts = ErrorFacts {
        proxy_type: exception.proxy_type.clone(),
        message: message.to_owned(),
        class: semantic_class(exception),
        status_code: error_code.parse::<u16>().ok(),
        rate_limit_type: dimension,
    };
    normalizer.normalize(Some(&facts)).map(str::to_owned)
}

fn client_disconnected(metadata: &MetadataMap) -> Result<bool, ErrorInformationError> {
    match metadata.get("client_disconnected") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(flag)) => Ok(*flag),
        Some(_) => Err(ErrorInformationError::ClientDisconnectedType),
    }
}

fn client_disconnect_information(
    metadata: &MetadataMap,
) -> Result<ErrorInformation, ErrorInformationError> {
    let Some(Json::Object(fields)) = metadata.get("error_information") else {
        return Ok(ErrorInformation {
            error_code: Some(Some("499".to_owned())),
            error_class: Some(Some("ClientDisconnected".to_owned())),
            llm_provider: None,
            traceback: None,
            error_message: Some(Some(CLIENT_DISCONNECTED_MESSAGE.to_owned())),
            error_provider_request_id: None,
            error_rate_limit_category: None,
            error_rate_limit_type: None,
            error_budget_entity_type: None,
            error_budget_entity_id: None,
            normalized_error: None,
            error_budget_limit: None,
            error_budget_spend: None,
        });
    };
    Ok(decode("error_information", fields.clone())?)
}

fn redact(policy: Option<&Redactor>, text: &str) -> String {
    let Some(redactor) = policy else {
        return text.to_owned();
    };
    redactor.redact_string(text)
}

mod tests;

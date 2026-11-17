use std::collections::BTreeMap;
use std::str::FromStr;

use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::{
    BudgetExceededDetails, GuardrailDetails, RateLimitDetails, ResponseDetails,
    SchemaValidationDetails,
};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_router::fallback_observations::details::fallback_error_detail;
use betterlite_router::fallback_observations::error_info::fallback_error_info;
use rust_decimal::Decimal;

use super::super::wire::{DetailCase, DetailFound};
use crate::router_probes::error::OperationError;

const MODEL: &str = "gpt-4";
const PROVIDER: &str = "openai";

pub(super) fn details(cases: &[DetailCase]) -> Result<Vec<DetailFound>, OperationError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|source| {
        OperationError::InvalidRequest {
            message: format!("the fallback detail redactor could not be built: {source}"),
        }
    })?;
    cases
        .iter()
        .map(|case| {
            let class = error_class(case)?;
            Ok(DetailFound {
                info: fallback_error_info(&class),
                detail: fallback_error_detail(&redactor, &class),
            })
        })
        .collect()
}

// the adapter builds the ErrorClass the router would classify for the exception the case names, message text included
fn error_class(case: &DetailCase) -> Result<ErrorClass, OperationError> {
    match case {
        DetailCase::RateLimit {
            message,
            body_param,
        } => Ok(rate_limit(message, body_param.as_deref())),
        DetailCase::BadRequest {
            message,
            body_type,
            body_param,
        } => Ok(bad_request(
            message,
            body_type.as_deref(),
            body_param.as_deref(),
        )),
        DetailCase::Budget {
            message,
            current_cost,
            max_budget,
        } => budget(message.as_deref(), current_cost, max_budget),
        DetailCase::Guardrail {
            message,
            guardrail_name,
        } => Ok(guardrail(message, guardrail_name.as_deref())),
        DetailCase::JsonSchema {
            raw_response,
            json_schema,
        } => Ok(json_schema(raw_response, json_schema)),
        DetailCase::Plain { message, .. } => Ok(ErrorClass::Unclassified(Box::new(context(
            message.to_owned(),
        )))),
    }
}

pub(super) fn context(message: String) -> ErrorContext {
    ErrorContext {
        message,
        model: Some(MODEL.to_owned()),
        provider: Some(PROVIDER.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn rate_limit(message: &str, body_param: Option<&str>) -> ErrorClass {
    // RateLimitError.__init__ puts its class name in front of the message it was given
    let mut details = context(format!("litellm.RateLimitError: {message}"));
    details.body_param = body_param.map(str::to_owned);
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: details,
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

fn bad_request(message: &str, body_type: Option<&str>, body_param: Option<&str>) -> ErrorClass {
    // BadRequestError.__init__ does the same with its own class name
    let mut details = context(format!("litellm.BadRequestError: {message}"));
    details.body_type = body_type.map(str::to_owned);
    details.body_param = body_param.map(str::to_owned);
    ErrorClass::BadRequest(Box::new(ResponseDetails::from(details)))
}

// BudgetExceededError reads a missing or empty message as absent and renders the costs themselves
fn budget(
    message: Option<&str>,
    current_cost: &str,
    max_budget: &str,
) -> Result<ErrorClass, OperationError> {
    Ok(ErrorClass::BudgetExceeded(Box::new(
        BudgetExceededDetails {
            current_cost: decimal(current_cost)?,
            max_budget: decimal(max_budget)?,
            message: message.map(str::to_owned),
            // BudgetExceededError's llm_provider defaults to the empty string
            provider: String::new(),
            entity_type: None,
            entity_id: None,
            status_code: 422,
        },
    )))
}

fn decimal(text: &str) -> Result<Decimal, OperationError> {
    Decimal::from_str(text).map_err(|source| OperationError::InvalidRequest {
        message: format!("{text} is not a decimal cost: {source}"),
    })
}

fn guardrail(message: &str, guardrail_name: Option<&str>) -> ErrorClass {
    let name = guardrail_name.unwrap_or("None");
    ErrorClass::GuardrailRaised(Box::new(GuardrailDetails {
        guardrail_name: guardrail_name.map(str::to_owned),
        message: format!("Guardrail raised an exception, Guardrail: {name}, Message: {message}"),
        status_code: 400,
        blocked_content: false,
    }))
}

fn json_schema(raw_response: &str, schema: &str) -> ErrorClass {
    ErrorClass::JsonSchemaValidation(Box::new(SchemaValidationDetails {
        model: MODEL.to_owned(),
        provider: PROVIDER.to_owned(),
        raw_response: raw_response.to_owned(),
        schema: schema.to_owned(),
    }))
}

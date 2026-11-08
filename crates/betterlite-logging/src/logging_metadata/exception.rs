use betterlite_error::normalize::class::Class;
use betterlite_error::rate_limit::RateLimitDimension;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::value::StrDeserializer;

use crate::fields::payload::records::DecimalValue;

const PROVIDER_REQUEST_ID_HEADERS: [&str; 7] = [
    "x-amzn-requestid",
    "x-request-id",
    "request-id",
    "x-ms-request-id",
    "apim-request-id",
    "x-goog-request-id",
    "cf-ray",
];

// What get_error_information reads off the exception, with the Python traceback formatted by the caller
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExceptionFacts {
    #[serde(default)]
    pub class_name: String,
    #[serde(default)]
    pub class: Option<Class>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub status_code: Option<u16>,
    #[serde(default)]
    pub response_status_code: Option<u16>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub rate_limit_type: Option<String>,
    #[serde(default)]
    pub budget_entity_type: Option<String>,
    #[serde(default)]
    pub budget_entity_id: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub budget_limit: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub budget_spend: Option<Decimal>,
    #[serde(default)]
    pub proxy_type: Option<String>,
    #[serde(default)]
    pub headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub response_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub litellm_response_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub traceback_lines: Vec<String>,
}

pub(crate) fn exception_code(exception: &ExceptionFacts) -> String {
    if let Some(code) = exception
        .code
        .as_deref()
        .filter(|code| !code.is_empty() && *code != "None")
    {
        return code.to_owned();
    }
    if let Some(status) = exception.status_code {
        return status.to_string();
    }
    if let Some(status) = exception.response_status_code {
        return status.to_string();
    }
    String::new()
}

pub(crate) fn budget_fields(
    exception: &ExceptionFacts,
) -> (
    Option<String>,
    Option<String>,
    Option<DecimalValue>,
    Option<DecimalValue>,
) {
    if !matches!(semantic_class(exception), Class::BudgetExceededError) {
        return (None, None, None, None);
    }
    (
        exception.budget_entity_type.clone(),
        exception.budget_entity_id.clone(),
        exception.budget_limit.map(DecimalValue),
        exception.budget_spend.map(DecimalValue),
    )
}

pub(crate) fn provider_request_id(exception: &ExceptionFacts) -> Option<String> {
    let sources = [
        exception.headers.as_ref(),
        exception.response_headers.as_ref(),
        exception.litellm_response_headers.as_ref(),
    ];
    for expected in PROVIDER_REQUEST_ID_HEADERS {
        let found = sources.iter().flatten().find_map(|headers| {
            headers
                .iter()
                .find(|(name, value)| name.eq_ignore_ascii_case(expected) && !value.is_empty())
        });
        if let Some((_name, value)) = found {
            return Some(value.clone());
        }
    }
    None
}

const RATE_LIMIT_CATEGORIES: [&str; 4] = [
    "vendor_rate_limit",
    "vendor_batch_rate_limit",
    "litellm_rate_limit",
    "litellm_batch_rate_limit",
];

const RATE_LIMIT_TYPES: [&str; 5] = [
    "requests",
    "tokens",
    "concurrent_requests",
    "budget",
    "max_iterations",
];

pub(crate) fn validated_category(raw: Option<&str>) -> Option<String> {
    let value = raw?;
    RATE_LIMIT_CATEGORIES
        .contains(&value)
        .then_some(value.to_owned())
}

pub(crate) fn validated_type(raw: Option<&str>) -> Option<String> {
    let value = raw?;
    RATE_LIMIT_TYPES
        .contains(&value)
        .then_some(value.to_owned())
}

pub(crate) fn validated_dimension(raw: Option<&str>) -> Option<RateLimitDimension> {
    match raw? {
        "requests" => Some(RateLimitDimension::Requests),
        "tokens" => Some(RateLimitDimension::Tokens),
        "concurrent_requests" => Some(RateLimitDimension::ConcurrentRequests),
        "budget" => Some(RateLimitDimension::Budget),
        "max_iterations" => Some(RateLimitDimension::MaxIterations),
        _ => None,
    }
}

pub(crate) fn semantic_class(exception: &ExceptionFacts) -> Class {
    if let Some(class) = exception.class {
        return class;
    }
    let deserializer = StrDeserializer::<serde_json::Error>::new(&exception.class_name);
    Class::deserialize(deserializer).unwrap_or(Class::Other)
}

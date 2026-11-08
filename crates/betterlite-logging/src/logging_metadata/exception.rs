use betterlite_error::normalize::class::Class;
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
    pub class: Option<Class>,
    pub message: Option<String>,
    pub code: Option<String>,
    pub status_code: Option<u16>,
    pub response_status_code: Option<u16>,
    pub llm_provider: Option<String>,
    pub category: Option<String>,
    pub rate_limit_type: Option<String>,
    pub budget_entity_type: Option<String>,
    pub budget_entity_id: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub budget_limit: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub budget_spend: Option<Decimal>,
    pub proxy_type: Option<String>,
    pub headers: Option<IndexMap<String, String>>,
    pub response_headers: Option<IndexMap<String, String>>,
    pub litellm_response_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub traceback_lines: Vec<String>,
}

pub(crate) fn exception_code(exception: &ExceptionFacts) -> String {
    exception
        .code
        .as_deref()
        .filter(|code| !code.is_empty() && *code != "None")
        .map(str::to_owned)
        .or_else(|| {
            let status = exception.status_code.or(exception.response_status_code);
            status.map(|code| code.to_string())
        })
        .unwrap_or_default()
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
    PROVIDER_REQUEST_ID_HEADERS.iter().find_map(|expected| {
        sources.iter().flatten().find_map(|headers| {
            let named = headers
                .iter()
                .find(|(name, value)| name.eq_ignore_ascii_case(expected) && !value.is_empty());
            named.map(|(_name, value)| value.clone())
        })
    })
}

const RATE_LIMIT_CATEGORIES: [&str; 4] = [
    "vendor_rate_limit",
    "vendor_batch_rate_limit",
    "litellm_rate_limit",
    "litellm_batch_rate_limit",
];

pub(crate) fn validated_category(raw: Option<&str>) -> Option<String> {
    let value = raw?;
    RATE_LIMIT_CATEGORIES
        .contains(&value)
        .then_some(value.to_owned())
}

pub(crate) fn semantic_class(exception: &ExceptionFacts) -> Class {
    exception.class.unwrap_or_else(|| {
        let deserializer = StrDeserializer::<serde_json::Error>::new(&exception.class_name);
        Class::deserialize(deserializer).unwrap_or(Class::Other)
    })
}

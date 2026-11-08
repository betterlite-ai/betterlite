use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::fields::status::LoggingStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MockStatusFields {
    pub llm_api_status: LoggingStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ModelMapInformation {
    pub model: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MockStandardLoggingPayload {
    pub id: &'static str,
    pub trace_id: &'static str,
    pub call_type: &'static str,
    pub stream: bool,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_cost: Decimal,
    pub status: LoggingStatus,
    pub status_fields: MockStatusFields,
    pub custom_llm_provider: &'static str,
    pub total_tokens: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    #[serde(
        rename = "startTime",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub start_time: Decimal,
    #[serde(rename = "endTime", with = "rust_decimal::serde::arbitrary_precision")]
    pub end_time: Decimal,
    #[serde(
        rename = "completionStartTime",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub completion_start_time: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_time: Decimal,
    pub model_map_information: ModelMapInformation,
    pub model: &'static str,
    pub model_id: Option<&'static str>,
    pub model_group: Option<&'static str>,
    pub api_base: &'static str,
    pub metadata: BTreeMap<String, Json>,
    pub cache_hit: Option<bool>,
    pub cache_key: Option<&'static str>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub saved_cache_cost: Decimal,
    pub request_tags: &'static [Json],
    pub end_user: Option<&'static str>,
    pub requester_ip_address: Option<&'static str>,
    pub messages: Option<Json>,
    pub response: Option<Json>,
    pub error_str: Option<&'static str>,
    pub error_information: Option<Json>,
    pub model_parameters: BTreeMap<String, Json>,
    pub hidden_params: BTreeMap<String, Json>,
    pub guardrail_information: Option<Vec<Json>>,
    pub standard_built_in_tools_params: Option<Json>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MockStorageObject {
    pub kind: &'static str,
    pub name: &'static str,
}

pub const MOCK_LOGGING_PAYLOAD: MockStandardLoggingPayload = MockStandardLoggingPayload {
    id: "mock-request-id",
    trace_id: "mock-trace-id",
    call_type: "completion",
    stream: false,
    response_cost: Decimal::ZERO,
    status: LoggingStatus::Success,
    status_fields: MockStatusFields {
        llm_api_status: LoggingStatus::Success,
    },
    custom_llm_provider: "mock",
    total_tokens: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    start_time: Decimal::ZERO,
    end_time: Decimal::ZERO,
    completion_start_time: Decimal::ZERO,
    response_time: Decimal::ZERO,
    model_map_information: ModelMapInformation {
        model: "mock-model",
    },
    model: "mock-model",
    model_id: None,
    model_group: None,
    api_base: "https://api.mock.com",
    metadata: BTreeMap::new(),
    cache_hit: None,
    cache_key: None,
    saved_cache_cost: Decimal::ZERO,
    request_tags: &[],
    end_user: None,
    requester_ip_address: None,
    messages: None,
    response: None,
    error_str: None,
    error_information: None,
    model_parameters: BTreeMap::new(),
    hidden_params: BTreeMap::new(),
    guardrail_information: None,
    standard_built_in_tools_params: None,
};

pub const MOCK_STORAGE_OBJECT: MockStorageObject = MockStorageObject {
    kind: "storage#object",
    name: "mock-object",
};

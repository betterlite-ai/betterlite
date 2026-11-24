use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message as CountMessage;
use rust_decimal::Decimal;

use super::{CONTENT_STOP, DELTA, MESSAGE_STOP, START, TEXT_DELTA, TEXT_START};
use crate::pass_through_logging::response_costing::{ResponseCostRequest, ResponseCostService};

pub(super) fn cut_stream_bytes() -> Vec<u8> {
    cut_stream_text().into_bytes()
}

pub(super) fn cut_stream_text() -> String {
    [START, TEXT_START, TEXT_DELTA].concat()
}

pub(super) fn completed_stream_bytes() -> Vec<u8> {
    [
        START,
        TEXT_START,
        TEXT_DELTA,
        CONTENT_STOP,
        DELTA,
        MESSAGE_STOP,
    ]
    .concat()
    .into_bytes()
}

pub(super) fn recorded_usage(call_log: &LoggingState) -> &JsonObject {
    match call_log.model_call_details.get("combined_usage_object") {
        Some(Json::Object(fields)) => fields,
        None | Some(_) => panic!("the failure recorded no usage"),
    }
}

pub(super) fn recorded_cost(call_log: &LoggingState) -> Option<f64> {
    match call_log.model_call_details.get("response_cost") {
        Some(Json::Number(number)) => number.as_f64(),
        None | Some(_) => None,
    }
}

pub(super) fn number(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64(),
        None | Some(_) => None,
    }
}

pub(super) fn request_body(model: &str) -> JsonObject {
    let mut fields = JsonObject::new();
    fields.insert("model".to_owned(), Json::Text(model.to_owned()));
    fields
}

pub(super) struct FakeCost {
    price: Option<Decimal>,
}

impl FakeCost {
    pub(super) fn priced(price: Decimal) -> Self {
        Self { price: Some(price) }
    }

    pub(super) fn unpriced() -> Self {
        Self { price: None }
    }
}

impl ResponseCostService for FakeCost {
    fn completion_cost(&self, _request: &ResponseCostRequest<'_>) -> Result<Decimal, PortFailure> {
        self.price
            .ok_or_else(|| PortFailure::from("no price for this model"))
    }
}

pub(super) struct FixedCount(pub(super) i64);

#[async_trait]
impl UsageTokenCounter for FixedCount {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(self.0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(self.0)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[CountMessage]>,
    ) -> Result<i64, CountError> {
        Ok(self.0)
    }
}

pub(super) struct FailingCount;

#[async_trait]
impl UsageTokenCounter for FailingCount {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Err(CountError::MissingInput)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Err(CountError::MissingInput)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[CountMessage]>,
    ) -> Result<i64, CountError> {
        Err(CountError::MissingInput)
    }
}

pub(super) struct RecordingCount {
    count: i64,
    texts: Mutex<Vec<String>>,
    models: Mutex<Vec<String>>,
}

impl RecordingCount {
    pub(super) fn new(count: i64) -> Self {
        Self {
            count,
            texts: Mutex::new(Vec::new()),
            models: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn texts(&self) -> Vec<String> {
        self.texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn models(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl UsageTokenCounter for RecordingCount {
    async fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        self.texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text.to_owned());
        Ok(self.count)
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        self.texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text.to_owned());
        Ok(self.count)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[CountMessage]>,
    ) -> Result<i64, CountError> {
        Ok(self.count)
    }
}

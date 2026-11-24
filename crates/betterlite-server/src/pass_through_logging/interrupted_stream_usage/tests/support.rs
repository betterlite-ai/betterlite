use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject as ShapedObject, JsonValue as ShapedValue,
};
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::{
    CacheCreationTokenDetails, CompletionTokensDetails, PromptTokensDetails, ServerToolUse,
    Usage as WireUsage,
};
use betterlite_streaming::streamed_response_assembly::response::{
    ChatChoice, ChatMessage, ChatResponse, Response,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message as CountMessage;
use rust_decimal::Decimal;
use serde_json::Number;

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

pub(super) fn detail_counts_response() -> Response {
    Response::Chat(Box::new(ChatResponse {
        id: "chatcmpl-b3632".to_owned(),
        object: "chat.completion".to_owned(),
        created: 1_700_000_000,
        model: "claude-test".to_owned(),
        system_fingerprint: None,
        service_tier: None,
        choices: vec![detail_counts_choice()],
        usage: detail_counts_usage(),
        hidden_params: ShapedObject::new(),
    }))
}

fn detail_counts_choice() -> ChatChoice {
    ChatChoice {
        index: 0,
        message: ChatMessage {
            content: Some("hello".to_owned()),
            role: "assistant".to_owned(),
            tool_calls: None,
            function_call: None,
            provider_specific_fields: None,
            reasoning_content: None,
            thinking_blocks: None,
            annotations: None,
            audio: None,
            images: None,
        },
        finish_reason: "stop".to_owned(),
        provider_specific_fields: None,
    }
}

fn detail_counts_usage() -> WireUsage {
    let (prompt_extra, completion_extra) = detail_extras();
    WireUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        cache_creation_input_tokens: Some(2),
        cache_read_input_tokens: Some(3),
        completion_tokens_details: Some(CompletionTokensDetails {
            reasoning_tokens: Some(1),
            text_tokens: Some(4),
            image_tokens: Some(0),
            audio_tokens: None,
            extra: completion_extra,
        }),
        prompt_tokens_details: Some(PromptTokensDetails {
            cached_tokens: Some(7),
            web_search_requests: Some(1),
            google_maps_grounding_requests: Some(2),
            cache_write_tokens: Some(3),
            cache_creation_tokens: Some(4),
            cache_creation_token_details: Some(CacheCreationTokenDetails {
                ephemeral_5m_input_tokens: Some(5),
                ephemeral_1h_input_tokens: Some(6),
            }),
            extra: prompt_extra,
        }),
        server_tool_use: Some(ServerToolUse {
            web_search_requests: Some(1),
            tool_search_requests: Some(2),
            browser_open_requests: Some(3),
        }),
        cost: Some(Decimal::new(25, 2)),
        inference_geo: Some("us".to_owned()),
        speed: Some("fast".to_owned()),
    }
}

fn detail_extras() -> (ShapedObject, ShapedObject) {
    let mut prompt_extra = ShapedObject::new();
    prompt_extra.insert(
        "custom_prompt".to_owned(),
        ShapedValue::Number(Number::from(9_i64)),
    );
    let mut completion_extra = ShapedObject::new();
    completion_extra.insert("custom_completion".to_owned(), ShapedValue::Bool(true));
    (prompt_extra, completion_extra)
}

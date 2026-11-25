use std::str::FromStr as _;

use async_trait::async_trait;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject as ShapedObject, JsonValue as ShapedValue,
};
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::{
    CompletionTokensDetails, Usage as WireUsage,
};
use betterlite_streaming::streamed_response_assembly::response::{
    ChatChoice, ChatMessage, ChatResponse, Response,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message as CountMessage;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::Number;

use super::{Chunk, InterruptedStreamUsageError};

// Every field a case can set on a chat response; the counts are the usage the stream already reported.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChatSpec {
    pub content: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub has_details: bool,
    pub text_tokens: Option<i64>,
    pub cost: Option<String>,
    pub hidden_response_cost: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UsageSummary {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub text_tokens: Option<i64>,
    pub cost: Option<Decimal>,
    pub hidden_response_cost: Option<Decimal>,
}

#[derive(Debug)]
pub struct RecoveryOutcome {
    pub changed: bool,
    pub usage: UsageSummary,
}

#[derive(Debug)]
pub struct ClearOutcome {
    pub cost_cleared: bool,
    pub hidden_cost_cleared: bool,
}

// ast-grep-ignore: no-padded-one-line-functions - the oracle and fuzz target pass raw bytes, and LiteLLM decodes them as strict UTF-8.
pub fn stream_was_interrupted(chunks: &[Vec<u8>]) -> Result<bool, InterruptedStreamUsageError> {
    let decoded = chunks_of(chunks);
    super::stream_was_interrupted(&decoded)
}

// LiteLLM patches token_counter in the oracle, so the seam counts with the same fixed number.
pub fn recover_with_fixed_count(
    spec: &ChatSpec,
    chunks: &[Vec<u8>],
    model: &str,
    count: i64,
) -> Result<RecoveryOutcome, InterruptedStreamUsageError> {
    let mut response = chat_response(spec);
    let decoded = chunks_of(chunks);
    // ast-grep-ignore: no-long-names
    let changed = super::recover_interrupted_stream_output_tokens(
        &mut response,
        &decoded,
        model,
        &FixedCount(count),
    )?;
    Ok(RecoveryOutcome {
        changed,
        usage: usage_summary(&response),
    })
}

#[must_use]
pub fn clear_placeholder_cost(spec: &ChatSpec) -> ClearOutcome {
    let mut response = chat_response(spec);
    super::clear_placeholder_cost(&mut response);
    let usage = usage_summary(&response);
    ClearOutcome {
        cost_cleared: usage.cost.is_none(),
        hidden_cost_cleared: usage.hidden_response_cost.is_none(),
    }
}

#[must_use]
pub fn chat_response(spec: &ChatSpec) -> Response {
    let details = spec.has_details.then_some(CompletionTokensDetails {
        reasoning_tokens: None,
        text_tokens: spec.text_tokens,
        image_tokens: None,
        audio_tokens: None,
        extra: ShapedObject::new(),
    });
    Response::Chat(Box::new(ChatResponse {
        id: "chatcmpl-b3632".to_owned(),
        object: "chat.completion".to_owned(),
        created: 1_700_000_000,
        model: "claude-test".to_owned(),
        system_fingerprint: None,
        service_tier: None,
        choices: vec![ChatChoice {
            index: 0,
            message: chat_message(spec),
            finish_reason: "stop".to_owned(),
            provider_specific_fields: None,
        }],
        usage: chat_usage(spec, details),
        hidden_params: hidden_params(spec),
    }))
}

fn chat_message(spec: &ChatSpec) -> ChatMessage {
    ChatMessage {
        content: spec.content.clone(),
        role: "assistant".to_owned(),
        tool_calls: None,
        function_call: None,
        provider_specific_fields: None,
        reasoning_content: None,
        thinking_blocks: None,
        annotations: None,
        audio: None,
        images: None,
    }
}

fn chat_usage(spec: &ChatSpec, details: Option<CompletionTokensDetails>) -> WireUsage {
    WireUsage {
        prompt_tokens: spec.prompt_tokens,
        completion_tokens: spec.completion_tokens,
        total_tokens: spec.total_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
        completion_tokens_details: details,
        prompt_tokens_details: None,
        server_tool_use: None,
        cost: spec
            .cost
            .as_deref()
            .and_then(|text| Decimal::from_str(text).ok()),
        inference_geo: None,
        speed: None,
    }
}

fn hidden_params(spec: &ChatSpec) -> ShapedObject {
    let mut hidden_params = ShapedObject::new();
    if let Some(text) = &spec.hidden_response_cost
        && let Some(hidden_amount) = Decimal::from_str(text).ok()
        && let Some(number) = hidden_amount.to_f64().and_then(Number::from_f64)
    {
        hidden_params.insert("response_cost".to_owned(), ShapedValue::Number(number));
    }
    hidden_params
}

#[must_use]
pub fn usage_summary(response: &Response) -> UsageSummary {
    match response {
        Response::Chat(chat) => UsageSummary {
            prompt_tokens: chat.usage.prompt_tokens,
            completion_tokens: chat.usage.completion_tokens,
            total_tokens: chat.usage.total_tokens,
            text_tokens: chat
                .usage
                .completion_tokens_details
                .as_ref()
                .and_then(|details| details.text_tokens),
            cost: chat.usage.cost,
            hidden_response_cost: hidden_cost(&chat.hidden_params),
        },
        Response::Text(text) => UsageSummary {
            prompt_tokens: text.usage.prompt_tokens,
            completion_tokens: text.usage.completion_tokens,
            total_tokens: text.usage.total_tokens,
            text_tokens: None,
            cost: None,
            hidden_response_cost: None,
        },
    }
}

fn chunks_of(chunks: &[Vec<u8>]) -> Vec<Chunk> {
    chunks
        .iter()
        .map(|bytes| Chunk::Bytes(bytes.clone()))
        .collect()
}

fn hidden_cost(hidden: &ShapedObject) -> Option<Decimal> {
    match hidden.get("response_cost") {
        Some(ShapedValue::Number(number)) => Decimal::from_str(&number.to_string()).ok(),
        None | Some(_) => None,
    }
}

struct FixedCount(i64);

#[async_trait]
impl UsageTokenCounter for FixedCount {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(self.0)
    }

    // The recovery only asks for the response-text count; these two keep the trait whole.
    #[cfg_attr(test, mutants::skip)]
    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(self.0)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[CountMessage]>,
    ) -> Result<i64, CountError> {
        Ok(self.0)
    }
}

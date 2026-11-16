#![expect(
    clippy::disallowed_types,
    reason = "a relayed chunk arrives as the provider's own JSON object and no typed model can hold every provider's keys"
)]

use async_trait::async_trait;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::stream::chunk_parser;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::streamed_response_assembly::cost::StreamCostCalculator;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{self, AssemblyError, Request};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, MessageContent, Part};
use betterlite_tokenizer::image::error::ImageError;
use betterlite_tokenizer::image::geometry::high_detail_upper_bound;
use betterlite_tokenizer::image::settings::Settings;
use betterlite_tokenizer::image::tokens::DEFAULT_BASE_TOKENS;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

mod tests;

const SSE_DATA_PREFIX: &str = "data: ";
const SSE_DATA_BARE_PREFIX: &str = "data:";
const IMAGE_URL_TYPE: &str = "image_url";
const HIGH_DETAIL: &str = "high";

#[derive(Debug, Error)]
pub enum RelayedCountError {
    #[error(transparent)]
    Counter(#[from] CountError),
    #[error("the high detail image upper bound could not be computed")]
    UpperBound(#[source] ImageError),
    #[error("the token count does not fit in a machine word")]
    Overflow,
}

// The builder's prompt count callback speaks CountError and turns any failure into zero, so only an overflow keeps its class.
impl From<RelayedCountError> for CountError {
    fn from(error: RelayedCountError) -> Self {
        match error {
            RelayedCountError::Counter(source) => source,
            RelayedCountError::UpperBound(ImageError::Overflow) | RelayedCountError::Overflow => {
                Self::Overflow
            }
            RelayedCountError::UpperBound(source) => {
                tracing::error!(
                    "The high detail image upper bound could not be computed: {source}"
                );
                Self::Overflow
            }
        }
    }
}

// The relay logging path is synchronous, so the caller binds a counter it can call from there.
pub trait RelayTokenCounter: Send + Sync {
    fn count_messages(&self, model: &str, messages: &[Message]) -> Result<i64, CountError>;
    fn count_text(&self, text: &str) -> Result<i64, CountError>;
    fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError>;
}

pub struct RelayAssemblyRequest<'request> {
    pub chunks: &'request [String],
    pub model: &'request str,
    pub messages: Option<&'request [Message]>,
    pub counter: &'request dyn RelayTokenCounter,
    pub costs: &'request dyn StreamCostCalculator,
    // The OpenAI chunk parser fills a chunk's id and created time from the caller's own sources.
    pub identify: fn() -> Uuid,
    pub clock: fn() -> i64,
}

pub fn count_relayed_prompt_tokens(
    counter: &dyn RelayTokenCounter,
    model: &str,
    messages: Option<&[Message]>,
) -> Result<i64, RelayedCountError> {
    let Some(list) = messages else {
        return Ok(0);
    };
    let mut local = Vec::with_capacity(list.len());
    let mut remote_images = 0_u64;
    for message in list {
        let (kept, remote) = without_remote_images(message);
        local.push(kept);
        remote_images = remote_images.saturating_add(remote);
    }
    let local_tokens = counter.count_messages(model, &local)?;
    let bound = high_detail_upper_bound(DEFAULT_BASE_TOKENS, &Settings::default())
        .map_err(RelayedCountError::UpperBound)?;
    let per_image = i64::try_from(bound).map_err(|_error| RelayedCountError::Overflow)?;
    let image_count = i64::try_from(remote_images).map_err(|_error| RelayedCountError::Overflow)?;
    let image_tokens = per_image
        .checked_mul(image_count)
        .ok_or(RelayedCountError::Overflow)?;
    local_tokens
        .checked_add(image_tokens)
        .ok_or(RelayedCountError::Overflow)
}

// LiteLLM drops the remote parts from the content list and leaves a content that is not a list alone.
fn without_remote_images(message: &Message) -> (Message, u64) {
    let Some(MessageContent::Parts(parts)) = &message.content else {
        return (message.clone(), 0);
    };
    let mut kept = Vec::with_capacity(parts.len());
    let mut remote = 0_u64;
    for part in parts {
        if is_remote_high_detail_image(part) {
            remote = remote.saturating_add(1);
        } else {
            kept.push(part.clone());
        }
    }
    let mut local = message.clone();
    local.content = Some(MessageContent::Parts(kept));
    (local, remote)
}

fn is_remote_high_detail_image(part: &Part) -> bool {
    let Part::Block(block) = part else {
        return false;
    };
    if block.kind != IMAGE_URL_TYPE {
        return false;
    }
    let Some(Json::Object(image_url)) = block.fields.get("image_url") else {
        return false;
    };
    let Some(Json::Text(url)) = image_url.get("url") else {
        return false;
    };
    let lowered = url.to_lowercase();
    if !(lowered.starts_with("http://") || lowered.starts_with("https://")) {
        return false;
    }
    matches!(image_url.get("detail"), Some(Json::Text(detail)) if detail == HIGH_DETAIL)
}

async fn assemble<L, C>(
    request: &RelayAssemblyRequest<'_>,
    chunks: &[Chunk],
    hooks: &VerboseHooks<L, C>,
) -> Result<Option<Response>, AssemblyError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let counter = RelayedCounter {
        counter: request.counter,
    };
    let count_prompt_tokens = || {
        count_relayed_prompt_tokens(request.counter, request.model, request.messages)
            .map_err(CountError::from)
    };
    let builder_request = Request {
        chunks: Some(chunks),
        messages: request.messages,
        count_prompt_tokens: Some(&count_prompt_tokens),
        logging: None,
        costs: request.costs,
        counter: &counter,
        metadata: None,
        now: (request.clock)(),
    };
    streamed_response_assembly::stream_chunk_builder(&builder_request, hooks).await
}

struct RelayedCounter<'counter> {
    counter: &'counter dyn RelayTokenCounter,
}

#[async_trait]
impl UsageTokenCounter for RelayedCounter<'_> {
    async fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError> {
        self.counter.count_response_text(model, text)
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        self.counter.count_text(text)
    }

    async fn count_messages(
        &self,
        model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        count_relayed_prompt_tokens(self.counter, model, messages).map_err(CountError::from)
    }
}

// LiteLLM parses every relayed chunk with the OpenAI iterator and keeps the ones that parsed.
fn parse_chunks(request: &RelayAssemblyRequest<'_>) -> Vec<Chunk> {
    request
        .chunks
        .iter()
        .filter_map(|raw| parse_chunk(raw, request.identify, request.clock))
        .collect()
}

fn parse_chunk(raw: &str, identify: fn() -> Uuid, clock: fn() -> i64) -> Option<Chunk> {
    let value: Value = serde_json::from_str(strip_sse_data_prefix(raw)).ok()?;
    let fields = value.as_object()?;
    // An empty object is falsy in LiteLLM, so it never reaches the chunk parser.
    if fields.is_empty() {
        return None;
    }
    let mut reply = chunk_parser(&value, identify, clock).ok()?;
    keep_reasoning_content(&value, &mut reply);
    relay_chunk(&value, &reply)
}

// LiteLLM's relay parser maps reasoning onto reasoning_content only when reasoning_content is None, while the streaming parser lets reasoning win, so the raw value goes back.
fn keep_reasoning_content(raw: &Value, reply: &mut StreamReply) {
    let Some(Value::Array(raw_choices)) = raw.get("choices") else {
        return;
    };
    for (choice, raw_choice) in reply.choices.iter_mut().zip(raw_choices) {
        if let Some(Value::String(text)) = raw_choice.pointer("/delta/reasoning_content") {
            choice.delta.reasoning_content = Some(text.clone());
        }
    }
}

fn relay_chunk(raw: &Value, reply: &StreamReply) -> Option<Chunk> {
    let mut value = serde_json::to_value(reply).ok()?;
    let Value::Object(fields) = &mut value else {
        return None;
    };
    // The parsed reply keeps no system_fingerprint, while LiteLLM's chunk record carries it; a null reads back as absent.
    if let Some(fingerprint) = raw.get("system_fingerprint") {
        fields.insert("system_fingerprint".to_owned(), fingerprint.clone());
    }
    serde_json::from_value(value).ok()
}

fn strip_sse_data_prefix(chunk: &str) -> &str {
    if let Some(stripped) = chunk.strip_prefix(SSE_DATA_PREFIX) {
        return stripped;
    }
    chunk.strip_prefix(SSE_DATA_BARE_PREFIX).unwrap_or(chunk)
}

pub async fn complete_streaming_response<L, C>(
    request: &RelayAssemblyRequest<'_>,
    hooks: &VerboseHooks<L, C>,
) -> Option<Response>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let chunks = parse_chunks(request);
    if chunks.is_empty() {
        tracing::warn!("No valid chunks found in streaming response");
        return None;
    }
    match assemble(request, &chunks, hooks).await {
        Ok(response) => response,
        Err(error) => {
            tracing::error!("Error building complete streaming response: {error}");
            None
        }
    }
}

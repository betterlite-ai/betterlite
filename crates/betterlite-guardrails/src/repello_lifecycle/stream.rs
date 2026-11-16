use std::collections::BTreeMap;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{
    AssemblyError, Request, stream_chunk_builder,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::repello_content::response::{ResponseSource, extract_response_text};

use super::RepelloAILifecycle;

pub struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        false
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, _payload: &Payload) -> bool {
        false
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

pub struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

// The guardrail only needs the assembled text, so no value these services answer reaches its output,
// and a mutant of their bodies reads back into nothing.
struct NoStreamCosts;

impl StreamCostCalculator for NoStreamCosts {
    #[cfg_attr(test, mutants::skip)]
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    #[cfg_attr(test, mutants::skip)]
    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

struct NoTokenCounter;

#[async_trait]
impl UsageTokenCounter for NoTokenCounter {
    #[cfg_attr(test, mutants::skip)]
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        Ok(0)
    }
}

static NO_COSTS: NoStreamCosts = NoStreamCosts;
static NO_COUNTER: NoTokenCounter = NoTokenCounter;

// What LiteLLM's stream_chunk_builder gives back for the buffered chunks; kind is its type name.
pub(super) struct Assembled {
    pub(super) text: Option<String>,
    pub(super) kind: &'static str,
}

#[derive(Debug, Error)]
pub(super) enum StreamScanError {
    #[error("a streamed chunk could not be read")]
    Chunk(#[source] serde_json::Error),
    #[error("the streamed response could not be assembled")]
    Assembly(#[from] AssemblyError),
}

impl From<StreamScanError> for GuardrailFailure {
    fn from(error: StreamScanError) -> Self {
        let detail = format!("RepelloAI Argus stream scan failed: {error}");
        Self::Other(detail)
    }
}

pub(super) async fn collect(mut response: mpsc::Receiver<GuardrailValue>) -> Vec<GuardrailValue> {
    let mut chunks = Vec::new();
    while let Some(chunk) = response.recv().await {
        chunks.push(chunk);
    }
    chunks
}

pub(super) async fn replay(chunks: Vec<GuardrailValue>, output: mpsc::Sender<GuardrailValue>) {
    for chunk in chunks {
        if output.send(chunk).await.is_err() {
            break;
        }
    }
}

pub(super) async fn relay(
    mut response: mpsc::Receiver<GuardrailValue>,
    output: mpsc::Sender<GuardrailValue>,
) {
    while let Some(chunk) = response.recv().await {
        if output.send(chunk).await.is_err() {
            break;
        }
    }
}

pub(super) fn parse_chunks(chunks: &[GuardrailValue]) -> Result<Vec<Chunk>, StreamScanError> {
    let mut parsed = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let encoded = serde_json::to_value(chunk).map_err(StreamScanError::Chunk)?;
        parsed.push(serde_json::from_value(encoded).map_err(StreamScanError::Chunk)?);
    }
    Ok(parsed)
}

pub(super) async fn assemble(
    guardrail: &RepelloAILifecycle,
    chunks: &[Chunk],
) -> Result<Assembled, StreamScanError> {
    let elapsed = (guardrail.clock)()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let now = i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX);
    let request = Request {
        chunks: Some(chunks),
        messages: None,
        count_prompt_tokens: None,
        logging: None,
        costs: &NO_COSTS,
        counter: &NO_COUNTER,
        metadata: None,
        now,
    };
    let assembled = stream_chunk_builder(&request, &guardrail.hooks).await?;
    match assembled {
        Some(Response::Chat(chat)) => {
            let encoded = serde_json::to_value(&chat).map_err(StreamScanError::Chunk)?;
            let body: BTreeMap<String, GuardrailValue> =
                serde_json::from_value(encoded).map_err(StreamScanError::Chunk)?;
            let text = extract_response_text(&ResponseSource::Body(&body));
            Ok(Assembled {
                text,
                kind: "ModelResponse",
            })
        }
        Some(Response::Text(_text)) => Ok(Assembled {
            text: None,
            kind: "TextResponse",
        }),
        None => Ok(Assembled {
            text: None,
            kind: "NoneType",
        }),
    }
}

use std::sync::{Arc, Mutex};

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::passthrough_reconstruction::converter::{EventChunkConverter, UsageCalculator};
use crate::passthrough_reconstruction::error::ConvertError;
use crate::passthrough_reconstruction::events::{
    RawSseChunk, extract_sse_data, object_field, text_field,
};
use crate::stream_usage_assembly::UsageTokenCounter;
use crate::stream_usage_assembly::wire::Usage;
use crate::streamed_response_assembly::cost::{CostError, CostInput, StreamCostCalculator};
use crate::streamed_response_assembly::records::Chunk;

pub(super) const FIXED_NOW: i64 = 1_700_000_000;
pub(super) const FIXED_MODEL: &str = "claude-3-5-sonnet";

pub(super) const fn fixed_id() -> Uuid {
    Uuid::from_u128(1)
}

pub(super) fn text_chunk(source: &str) -> RawSseChunk {
    RawSseChunk::Text(source.to_owned())
}

#[derive(Debug, thiserror::Error)]
#[error("scripted failure")]
pub(super) struct ScriptFailure;

fn chunk(raw: &str) -> Chunk {
    serde_json::from_str(raw).expect("fixture chunk parses")
}

pub(super) fn base_chunk() -> Chunk {
    chunk(&format!(
        r#"{{"id":"chatcmpl-fixture","object":"chat.completion.chunk","created":{FIXED_NOW},"model":"{FIXED_MODEL}"}}"#
    ))
}

pub(super) fn delta_chunk(text: &str) -> Chunk {
    let encoded = serde_json::to_string(text).expect("fixture delta text encodes");
    chunk(&format!(
        r#"{{"id":"chatcmpl-fixture","object":"chat.completion.chunk","created":{FIXED_NOW},"model":"{FIXED_MODEL}","choices":[{{"index":0,"delta":{{"content":{encoded}}}}}]}}"#
    ))
}

pub(super) fn finish_chunk(reason: &str) -> Chunk {
    let encoded = serde_json::to_string(reason).expect("fixture finish reason encodes");
    chunk(&format!(
        r#"{{"id":"chatcmpl-fixture","object":"chat.completion.chunk","created":{FIXED_NOW},"model":"{FIXED_MODEL}","choices":[{{"index":0,"finish_reason":{encoded}}}]}}"#
    ))
}

// The scripted converter answers the SSE shapes a text stream carries and can fail on a marker.
#[derive(Default)]
pub(super) struct ScriptedConverter {
    pub(super) seen: Vec<String>,
    pub(super) malformed: Option<&'static str>,
    pub(super) ended: Option<&'static str>,
    pub(super) failed: Option<&'static str>,
}

impl EventChunkConverter for ScriptedConverter {
    type Failure = ScriptFailure;

    fn convert_event(&mut self, event: &str) -> Result<Option<Chunk>, ConvertError<Self::Failure>> {
        self.seen.push(event.to_owned());
        if self.malformed.is_some_and(|marker| event.contains(marker)) {
            return Err(ConvertError::Malformed);
        }
        if self.ended.is_some_and(|marker| event.contains(marker)) {
            return Err(ConvertError::Ended);
        }
        if self.failed.is_some_and(|marker| event.contains(marker)) {
            return Err(ConvertError::Failed {
                source: ScriptFailure,
            });
        }
        Ok(scripted_chunk(event))
    }
}

fn scripted_chunk(event: &str) -> Option<Chunk> {
    let data = extract_sse_data(event)?;
    match text_field(&data, "type") {
        Some("message_start") => Some(base_chunk()),
        Some("content_block_delta") => {
            let text = object_field(&data, "delta")
                .and_then(|delta| text_field(delta, "text"))
                .unwrap_or_default();
            Some(delta_chunk(text))
        }
        Some("message_delta") => {
            let reason = object_field(&data, "delta")
                .and_then(|delta| text_field(delta, "stop_reason"))
                .unwrap_or("stop");
            Some(finish_chunk(reason))
        }
        _ => None,
    }
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    records: Mutex<Vec<Payload>>,
}

impl RecordingLogger {
    pub(super) fn messages(&self) -> Vec<String> {
        self.records
            .lock()
            .expect("fixture verbose log")
            .iter()
            .filter_map(|payload| match payload {
                Payload::String(text) => Some(text.clone()),
                Payload::Null
                | Payload::Bool(_)
                | Payload::Number(_)
                | Payload::Array(_)
                | Payload::Object(_) => None,
            })
            .collect()
    }
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .expect("fixture verbose log")
            .push(payload.clone());
        true
    }

    fn info(&self, payload: &Payload) -> bool {
        self.debug(payload)
    }

    fn error(&self, payload: &Payload) -> bool {
        self.debug(payload)
    }

    fn debug_enabled(&self) -> bool {
        true
    }
}

pub(super) struct RecordingConsole;

impl ConsoleSink for RecordingConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

pub(super) fn fixture_hooks() -> VerboseHooks<RecordingLogger, RecordingConsole> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles");
    VerboseHooks {
        logger: RecordingLogger::default(),
        console: RecordingConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    }
}

#[derive(Default)]
pub(super) struct FixtureCounter;

#[async_trait::async_trait]
impl UsageTokenCounter for FixtureCounter {
    async fn count_response_text(&self, _model: &str, text: &str) -> Result<i64, CountError> {
        Ok(i64::try_from(text.chars().count()).unwrap_or_default())
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        Ok(i64::try_from(text.chars().count()).unwrap_or_default())
    }

    async fn count_messages(
        &self,
        _model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        Ok(messages.map_or(0, |listed| i64::try_from(listed.len()).unwrap_or_default()))
    }
}

#[derive(Default)]
pub(super) struct FixtureCosts;

impl StreamCostCalculator for FixtureCosts {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

#[derive(Default)]
pub(super) struct FixtureCalculator {
    pub(super) seen_object: Mutex<Option<String>>,
    pub(super) seen_speed: Mutex<Option<String>>,
    pub(super) fail: bool,
}

impl FixtureCalculator {
    pub(super) fn seen_object(&self) -> String {
        self.seen_object
            .lock()
            .expect("fixture usage object")
            .clone()
            .expect("the calculator saw a usage object")
    }

    pub(super) fn seen_speed(&self) -> Option<String> {
        self.seen_speed.lock().expect("fixture usage speed").clone()
    }
}

impl UsageCalculator for FixtureCalculator {
    type Failure = ScriptFailure;

    fn calculate_usage(
        &self,
        usage_object: &Json,
        speed: Option<&str>,
    ) -> Result<Usage, Self::Failure> {
        let encoded = serde_json::to_string(usage_object).expect("fixture usage object encodes");
        *self.seen_object.lock().expect("fixture usage object") = Some(encoded);
        *self.seen_speed.lock().expect("fixture usage speed") = speed.map(str::to_owned);
        if self.fail {
            return Err(ScriptFailure);
        }
        Ok(Usage {
            prompt_tokens: 11,
            completion_tokens: 4,
            total_tokens: 15,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            server_tool_use: None,
            cost: None,
            inference_geo: None,
            speed: None,
        })
    }
}

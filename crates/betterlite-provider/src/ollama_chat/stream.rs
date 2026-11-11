use betterlite_prompts::arguments::json::Json;
use betterlite_streaming::stream_chunk_shaping::tokens::mapped_finish_reason;

use self::chunk::{DeltaFunction, DeltaToolCall, OllamaStreamChunk, StreamChoice, StreamDelta};
use self::wire::{RawChunk, decode_chunk};
use super::error::OllamaError;
use super::function::{FUNCTION_TYPE, WireToolCall};
use super::ids::IdSource;
use super::usage::Usage;

pub mod chunk;
pub mod wire;

const CHUNK_OBJECT: &str = "chat.completion.chunk";
const DATA_PREFIX: &str = "data: ";
const DATA_PREFIX_WITHOUT_SPACE: &str = "data:";
const DONE_MARKER: &str = "[DONE]";
const STOP_FINISH_REASON: &str = "stop";
const TOOL_CALLS_FINISH_REASON: &str = "tool_calls";
const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ReasoningState {
    #[default]
    Idle,
    Started,
    Finished,
}

#[derive(Debug, Default)]
pub struct OllamaChatIterator {
    reasoning: ReasoningState,
    seen_tool_calls: bool,
}

#[derive(Debug)]
pub enum OllamaStreamStep {
    Chunk(Box<OllamaStreamChunk>),
    Skipped,
    Done,
}

impl OllamaChatIterator {
    // B2214's _handle_string_chunk over one transport line, then chunk_parser
    pub fn next_chunk(
        &mut self,
        line: &str,
        now: i64,
        ids: &dyn IdSource,
    ) -> Result<OllamaStreamStep, OllamaError> {
        if line.trim().is_empty() {
            return Ok(OllamaStreamStep::Skipped);
        }
        if line.contains(DONE_MARKER) {
            return Ok(OllamaStreamStep::Done);
        }
        let payload = line
            .strip_prefix(DATA_PREFIX)
            .or_else(|| line.strip_prefix(DATA_PREFIX_WITHOUT_SPACE))
            .unwrap_or(line);
        let Ok(parsed) = serde_json::from_str::<Json>(payload) else {
            return Ok(OllamaStreamStep::Skipped);
        };
        if !parsed.is_truthy() {
            return Ok(OllamaStreamStep::Skipped);
        }
        let chunk = self.chunk_parser(payload.as_bytes(), now, ids)?;
        Ok(OllamaStreamStep::Chunk(Box::new(chunk)))
    }

    pub fn chunk_parser(
        &mut self,
        chunk: &[u8],
        now: i64,
        ids: &dyn IdSource,
    ) -> Result<OllamaStreamChunk, OllamaError> {
        let raw = decode_chunk(chunk)?;
        let delta = self.stream_delta(&raw.message, ids)?;
        let finish_reason = raw.done.then(|| self.finish_reason(&raw));
        let usage = done_chunk_usage(&raw);
        Ok(OllamaStreamChunk {
            id: ids.next_id(),
            object: String::from(CHUNK_OBJECT),
            created: now,
            model: raw.model,
            choices: vec![StreamChoice {
                index: 0,
                delta,
                finish_reason,
            }],
            usage,
        })
    }

    fn stream_delta(
        &mut self,
        message: &super::function::WireMessage,
        ids: &dyn IdSource,
    ) -> Result<StreamDelta, OllamaError> {
        let mut delta = StreamDelta::default();
        if let Some(calls) = message.tool_calls.as_ref() {
            self.seen_tool_calls = true;
            delta.tool_calls = Some(delta_tool_calls(calls, ids)?);
        }
        if let Some(thinking) = message.thinking.text().filter(|text| !text.is_empty()) {
            delta.reasoning_content = Some(thinking.to_owned());
            self.start_reasoning();
        }
        if let Some(content) = message.content.clone().filter(|text| !text.is_empty()) {
            self.content_delta(&mut delta, content);
        }
        Ok(delta)
    }

    fn start_reasoning(&mut self) {
        if self.reasoning == ReasoningState::Idle {
            self.reasoning = ReasoningState::Started;
        }
    }

    fn content_delta(&mut self, delta: &mut StreamDelta, mut content: String) {
        if self.reasoning == ReasoningState::Started {
            self.reasoning = ReasoningState::Finished;
        }
        if content.contains(THINK_OPEN) {
            content = content.replace(THINK_OPEN, "");
            self.start_reasoning();
        }
        if self.reasoning != ReasoningState::Idle && content.contains(THINK_CLOSE) {
            content = content.replace(THINK_CLOSE, "");
            self.reasoning = ReasoningState::Finished;
        }
        if self.reasoning == ReasoningState::Started {
            delta.reasoning_content = Some(content);
        } else {
            delta.content = Some(content);
        }
    }

    fn finish_reason(&self, raw: &RawChunk) -> String {
        if self.seen_tool_calls {
            return String::from(TOOL_CALLS_FINISH_REASON);
        }
        String::from(
            raw.done_reason
                .as_deref()
                .map_or(STOP_FINISH_REASON, mapped_finish_reason),
        )
    }
}

fn done_chunk_usage(chunk: &RawChunk) -> Option<Usage> {
    if !chunk.done {
        return None;
    }
    let prompt_tokens = chunk.prompt_eval_count?;
    let completion_tokens = chunk.eval_count?;
    Some(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
    })
}

fn delta_tool_calls(
    calls: &[WireToolCall],
    ids: &dyn IdSource,
) -> Result<Vec<DeltaToolCall>, OllamaError> {
    let mut converted = Vec::with_capacity(calls.len());
    let mut current_index: u32 = 0;
    for call in calls {
        let index = call.index.unwrap_or_else(|| {
            let assigned = current_index;
            current_index = current_index.saturating_add(1);
            assigned
        });
        let arguments = call.function.call_arguments()?;
        converted.push(DeltaToolCall {
            id: if arguments.complete {
                Some(ids.next_id())
            } else {
                call.id.clone()
            },
            kind: String::from(call.kind.as_deref().unwrap_or(FUNCTION_TYPE)),
            index,
            function: DeltaFunction {
                name: call.function.name.clone(),
                arguments: arguments.arguments,
            },
        });
    }
    Ok(converted)
}

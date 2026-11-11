use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};

use crate::ollama_chat::error::OllamaError;
use crate::ollama_chat::ids::IdSource;
use crate::ollama_chat::stream::chunk::StreamDelta;
use crate::ollama_chat::usage::Usage;

const CHUNK_FIELD: &str = "chunk";
const CHUNK_OBJECT: &str = "chat.completion.chunk";
const DONE_FIELD: &str = "done";
const ERROR_FIELD: &str = "error";
const EVAL_COUNT: &str = "eval_count";
const ID_PREFIX: &str = "chatcmpl-";
const PROMPT_EVAL_COUNT: &str = "prompt_eval_count";
const RESPONSE_FIELD: &str = "response";
const STOP_FINISH_REASON: &str = "stop";
const THINK_CLOSE: &str = "</think>";
const THINK_OPEN: &str = "<think>";
const THINKING_FIELD: &str = "thinking";

#[derive(Debug, Default)]
pub struct OllamaGenerateIterator {
    started_reasoning: bool,
    finished_reasoning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerateStreamStep {
    Done(GenerateDoneChunk),
    Chunk(GenerateStreamChunk),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateDoneChunk {
    pub text: String,
    pub is_finished: bool,
    pub finish_reason: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateStreamChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    pub choices: Vec<GenerateStreamChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateStreamChoice {
    pub index: u32,
    pub finish_reason: Option<String>,
    pub delta: StreamDelta,
}

impl OllamaGenerateIterator {
    // LiteLLM's _handle_string_chunk parses the line as JSON and hands it straight to chunk_parser
    pub fn chunk_parser(
        &mut self,
        bytes: &[u8],
        now: i64,
        ids: &dyn IdSource,
    ) -> Result<GenerateStreamStep, OllamaError> {
        let parsed = serde_json::from_slice::<Json>(bytes)
            .map_err(|source| OllamaError::Shape { source })?;
        let Json::Object(fields) = parsed else {
            return Err(OllamaError::ChunkField { field: CHUNK_FIELD });
        };
        if fields.contains_key(ERROR_FIELD) {
            return Err(error_chunk(&fields));
        }
        let done = match fields.get(DONE_FIELD) {
            Some(Json::Bool(done)) => *done,
            Some(_) | None => return Err(OllamaError::ChunkField { field: DONE_FIELD }),
        };
        if done {
            return Ok(GenerateStreamStep::Done(done_chunk(&fields)?));
        }
        let response = match fields.get(RESPONSE_FIELD) {
            Some(Json::Null) => None,
            Some(Json::Text(text)) => Some(text.as_str()),
            Some(_) | None => {
                return Err(OllamaError::ChunkField {
                    field: RESPONSE_FIELD,
                });
            }
        };
        let delta = self.delta(response.filter(|text| !text.is_empty()), &fields)?;
        Ok(GenerateStreamStep::Chunk(GenerateStreamChunk {
            id: format!("{ID_PREFIX}{}", ids.next_id()),
            object: String::from(CHUNK_OBJECT),
            created: now,
            model: None,
            usage: None,
            choices: vec![GenerateStreamChoice {
                index: 0,
                finish_reason: None,
                delta,
            }],
        }))
    }

    // the open tag strips before the close one, as LiteLLM's elif chain does, and the state spans chunks
    fn delta(&mut self, text: Option<&str>, fields: &Object) -> Result<StreamDelta, OllamaError> {
        let mut delta = StreamDelta::default();
        let Some(value) = text else {
            delta.reasoning_content = Some(thinking_text(fields)?);
            return Ok(delta);
        };
        let mut content = value.to_owned();
        if content.contains(THINK_OPEN) {
            content = content.replace(THINK_OPEN, "");
            self.started_reasoning = true;
        } else if content.contains(THINK_CLOSE) {
            content = content.replace(THINK_CLOSE, "");
            self.finished_reasoning = true;
        } else {
            // the chunk carries no tag, so the state stays as it was
        }
        if self.started_reasoning && !self.finished_reasoning {
            delta.reasoning_content = Some(content);
        } else {
            delta.content = Some(content);
        }
        Ok(delta)
    }
}

fn done_chunk(fields: &Object) -> Result<GenerateDoneChunk, OllamaError> {
    let prompt_tokens = count_field(fields, PROMPT_EVAL_COUNT)?;
    let completion_tokens = count_field(fields, EVAL_COUNT)?;
    let usage = match (prompt_tokens, completion_tokens) {
        (Some(found_prompt), Some(found_completion)) => Some(Usage {
            prompt_tokens: found_prompt,
            completion_tokens: found_completion,
            total_tokens: found_prompt.saturating_add(found_completion),
        }),
        _ => None,
    };
    Ok(GenerateDoneChunk {
        text: String::new(),
        is_finished: true,
        finish_reason: String::from(STOP_FINISH_REASON),
        usage,
    })
}

fn count_field(fields: &Object, name: &'static str) -> Result<Option<u64>, OllamaError> {
    match fields.get(name) {
        Some(Json::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or(OllamaError::ChunkField { field: name }),
        Some(Json::Null) | None => Ok(None),
        Some(_) => Err(OllamaError::ChunkField { field: name }),
    }
}

fn thinking_text(fields: &Object) -> Result<String, OllamaError> {
    match fields.get(THINKING_FIELD) {
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Null) | None => Ok(String::new()),
        Some(_) => Err(OllamaError::ChunkField {
            field: THINKING_FIELD,
        }),
    }
}

// LiteLLM raises a generic exception on an error chunk, which the gateway answers as a 500
fn error_chunk(fields: &Object) -> OllamaError {
    let message = match fields.get(ERROR_FIELD) {
        Some(Json::Text(text)) => text.clone(),
        Some(_) | None => String::from("the Ollama stream reported an error"),
    };
    OllamaError::Api {
        status_code: 500,
        message,
        headers: BTreeMap::new(),
    }
}

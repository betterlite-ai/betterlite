use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::model_armor::streams::assembly::Assembly;
use betterlite_core::guardrails::model_armor::streams::chunk::Chunk;
use betterlite_core::guardrails::model_armor::streams::errors::HttpError;
use betterlite_core::guardrails::model_armor::streams::hook::{
    StreamHook, StreamSettings, buffered_stream,
};
use betterlite_core::guardrails::model_armor::streams::surface::Surface;
use betterlite_core::guardrails::model_armor::streams::wire::{
    OutputParts, ScanFailure, StreamWire,
};
use betterlite_core::guardrails::streaming_rewrite::shapes::RequestData;
use betterlite_guardrails::anthropic_sse::error::AnthropicSseError;
use betterlite_guardrails::anthropic_sse::error_frames::anthropic_sse_error_frames;
use betterlite_guardrails::anthropic_sse::is_anthropic_sse_stream::is_anthropic_sse_stream;
use betterlite_guardrails::anthropic_sse::is_sse_error_stream::is_sse_error_stream;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::model_armor::decisions::model_response::{
    CallResponse, ModelResponse, ResponseContentError, extract_content_from_response,
};
use betterlite_guardrails::model_armor::decisions::wire::SanitizeResponse;
use betterlite_guardrails::model_armor::decisions::{
    build_api_error_detail, build_block_error_detail, build_logging_response,
    get_sanitized_content, has_deidentify_match, should_block_content,
};
use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod chat;
mod frames;
mod view;

const GUARDRAIL_NAME: &str = "model-armor";
const METADATA_KEY: &str = "metadata";
const LITELLM_METADATA_KEY: &str = "litellm_metadata";
const APPLIED_KEY: &str = "applied_guardrails";

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(transparent)]
struct Flag(bool);

#[derive(Debug, Deserialize)]
struct Input {
    chunks: Vec<ChunkInput>,
    mask_response_content: Flag,
    fail_on_error: Flag,
    sanitize_error_detail: Flag,
    answer_status: u16,
    answer_body: String,
}

// A chat or Responses chunk arrives as its JSON object, an Anthropic frame as the raw SSE text.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ChunkInput {
    Bytes(String),
    Json(GuardrailValue),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        runtime.block_on(evaluate(input))
    }
}

#[derive(Debug, thiserror::Error)]
enum WireError {
    #[error("the case answer is not valid JSON")]
    Decode(#[source] serde_json::Error),
    #[error(transparent)]
    Content(#[from] ResponseContentError),
    #[error(transparent)]
    Frames(#[from] AnthropicSseError),
    #[error("the assembled body has no {0}")]
    Missing(&'static str),
}

struct Wire {
    raw: GuardrailValue,
    typed: SanitizeResponse,
    api_error: Option<String>,
    sanitize_error_detail: bool,
    scanned: Mutex<Option<String>>,
}

impl Wire {
    fn new(input: &Input) -> Result<Self, WireError> {
        let api_error = if input.answer_status == 200 {
            None
        } else {
            Some(build_api_error_detail(
                input.answer_status,
                &input.answer_body,
                input.sanitize_error_detail.0,
            ))
        };
        let (raw, typed) = if api_error.is_some() {
            (GuardrailValue::Null, SanitizeResponse::default())
        } else {
            (
                serde_json::from_str(&input.answer_body).map_err(WireError::Decode)?,
                serde_json::from_str(&input.answer_body).map_err(WireError::Decode)?,
            )
        };
        Ok(Self {
            raw,
            typed,
            api_error,
            sanitize_error_detail: input.sanitize_error_detail.0,
            scanned: Mutex::new(None),
        })
    }

    fn scanned_content(&self) -> Option<String> {
        let scanned = self.scanned.lock().unwrap_or_else(PoisonError::into_inner);
        scanned.clone()
    }
}

impl StreamWire for Wire {
    type Failure = WireError;

    fn is_anthropic_sse(&self, chunks: &[Chunk]) -> bool {
        is_anthropic_sse_stream(&stream_items(chunks))
    }

    fn is_sse_error(&self, chunks: &[Chunk]) -> bool {
        is_sse_error_stream(&stream_items(chunks))
    }

    fn assemble_chat(&self, chunks: &[Chunk]) -> impl Future<Output = Option<Assembly>> + Send {
        ready(chat::assemble(chunks))
    }

    fn assemble_anthropic(
        &self,
        chunks: &[Chunk],
    ) -> impl Future<Output = Option<Assembly>> + Send {
        ready(frames::assemble(chunks))
    }

    fn model_text(&self, body: &GuardrailValue) -> Result<String, Self::Failure> {
        let plain = serde_json::to_value(body).map_err(WireError::Decode)?;
        let model: ModelResponse = serde_json::from_value(plain).map_err(WireError::Decode)?;
        extract_content_from_response(&CallResponse::Model(model)).map_err(WireError::from)
    }

    fn output_parts(&self, item: &GuardrailValue) -> OutputParts {
        view::output_parts(item)
    }

    fn scan(
        &self,
        content: &str,
    ) -> impl Future<Output = Result<GuardrailValue, ScanFailure<WireError>>> + Send {
        *self.scanned.lock().unwrap_or_else(PoisonError::into_inner) = Some(content.to_owned());
        let reply = match &self.api_error {
            Some(detail) => Err(ScanFailure::Api {
                detail: detail.clone(),
            }),
            None => Ok(self.raw.clone()),
        };
        ready(reply)
    }

    fn should_block(&self, _armor: &GuardrailValue, allow_sanitization: bool) -> bool {
        should_block_content(&self.typed, allow_sanitization)
    }

    fn sanitized_content(&self, _armor: &GuardrailValue) -> Option<String> {
        get_sanitized_content(&self.typed)
    }

    fn has_deidentify(&self, _armor: &GuardrailValue) -> bool {
        has_deidentify_match(&self.typed)
    }

    fn logging_response(&self, armor: &GuardrailValue) -> GuardrailValue {
        build_logging_response(armor, self.sanitize_error_detail)
    }

    fn block_detail(&self, message: &str, armor: &GuardrailValue) -> GuardrailValue {
        let detail = build_block_error_detail(message, armor, self.sanitize_error_detail);
        GuardrailValue::Object(detail.into_iter().collect())
    }

    fn emit_sanitized(
        &self,
        surface: Surface,
        body: &GuardrailValue,
    ) -> Result<Vec<Chunk>, Self::Failure> {
        match surface {
            Surface::ChatCompletions => chat::emit(body),
            Surface::AnthropicMessages => frames::emit(body),
            Surface::Responses | Surface::OpaqueSse => Ok(Vec::new()),
        }
    }

    fn anthropic_error_frames(&self, message: &str) -> Result<Vec<Chunk>, Self::Failure> {
        let frames = anthropic_sse_error_frames(message)?;
        Ok(frames.into_iter().map(Chunk::Bytes).collect())
    }

    fn responses_error_items(&self, error: &HttpError) -> Option<Vec<Chunk>> {
        frames::error_item(error).map(|item| vec![Chunk::Json(item)])
    }

    fn applied_guardrail(&self, data: &mut RequestData, guardrail_name: &str) {
        let key = if data.contains_key(LITELLM_METADATA_KEY) {
            LITELLM_METADATA_KEY
        } else {
            METADATA_KEY
        };
        let mut bucket = match data.remove(key) {
            Some(GuardrailValue::Object(fields)) => fields,
            _ => BTreeMap::new(),
        };
        let mut names = match bucket.remove(APPLIED_KEY) {
            Some(GuardrailValue::Array(names)) => names,
            _ => Vec::new(),
        };
        let known = names.iter().any(
            |name| matches!(name, GuardrailValue::Text(existing) if existing == guardrail_name),
        );
        if !known {
            names.push(GuardrailValue::Text(guardrail_name.to_owned()));
        }
        bucket.insert(APPLIED_KEY.to_owned(), GuardrailValue::Array(names));
        data.insert(key.to_owned(), GuardrailValue::Object(bucket));
    }
}

fn stream_items(chunks: &[Chunk]) -> Vec<StreamItem> {
    chunks
        .iter()
        .map(|chunk| match chunk {
            Chunk::Json(payload) => StreamItem::Json(payload.clone()),
            Chunk::Bytes(bytes) => StreamItem::Bytes(bytes.clone()),
        })
        .collect()
}

async fn evaluate(input: &Input) -> HelperResult {
    let wire = Wire::new(input).or_value_error()?;
    let mut data = RequestData::new();
    let settings = StreamSettings {
        fail_on_error: input.fail_on_error.0,
        mask_response_content: input.mask_response_content.0,
    };
    let chunks = input
        .chunks
        .iter()
        .map(|chunk| match chunk {
            ChunkInput::Bytes(text) => Chunk::Bytes(text.as_bytes().to_vec()),
            ChunkInput::Json(payload) => Chunk::Json(payload.clone()),
        })
        .collect();
    let items = buffered_stream(StreamHook {
        wire: &wire,
        request_data: &mut data,
        guardrail_name: GUARDRAIL_NAME,
        settings,
        chunks,
    })
    .await
    .or_value_error()?;
    let views = items.iter().map(view::item).collect::<Vec<Value>>();
    Ok(json!({
        "items": views,
        "scanned": wire.scanned_content(),
        "metadata": view::metadata(&data),
    }))
}

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_core::google_genai::metadata::GenAiStreamMetadata;
use betterlite_core::google_genai::provider::GenAiProvider;
use betterlite_core::google_genai::streaming::{AsyncGoogleGenAiStream, GoogleGenAiStream};
use betterlite_http::error::HttpError;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_models::decisions::HiddenParams;
use chrono::{DateTime, FixedOffset, Utc};
use futures_util::stream::{BoxStream, iter};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

const LINE_LIMIT: usize = 0x0010_0000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mode: Mode,
    provider: String,
    model: String,
    lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Sync,
    Async,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let metadata = stream_metadata(input);
        let body = line_bytes(&input.lines);
        match input.mode {
            Mode::Sync => sync_answer(metadata, body),
            Mode::Async => async_answer(metadata, body),
        }
    }
}

// LiteLLM's sync iterator hands out one event per next() call and never logs at the end
fn sync_answer(metadata: GenAiStreamMetadata, body: Vec<u8>) -> HelperResult {
    let mut stream = GoogleGenAiStream::new(std::iter::once(Ok(body)), metadata, LINE_LIMIT);
    let mut yielded = Vec::new();
    for item in &mut stream {
        yielded.push(item.or_value_error()?);
    }
    Ok(answer(&stream.metadata, &yielded, &stream.collected_chunks))
}

// LiteLLM's async iterator hands out one event per __anext__ call and logs at the end
fn async_answer(metadata: GenAiStreamMetadata, body: Vec<u8>) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    runtime.block_on(async {
        let source: BoxStream<'static, Result<Vec<u8>, HttpError>> =
            Box::pin(iter(std::iter::once(Ok::<Vec<u8>, HttpError>(body))));
        let mut stream =
            AsyncGoogleGenAiStream::new(source, metadata, None, Arc::new(WallClock), LINE_LIMIT);
        let mut yielded = Vec::new();
        while let Some(event) = stream.next_event().await.or_value_error()? {
            yielded.push(event);
        }
        Ok(answer(&stream.metadata, &yielded, &stream.collected_chunks))
    })
}

fn answer(metadata: &GenAiStreamMetadata, yielded: &[Vec<u8>], collected: &[Vec<u8>]) -> Value {
    json!({
        "provider": provider_spelling(metadata.provider),
        "model": metadata.model,
        "endpoint": endpoint_name(metadata.provider),
        "yielded": yielded.iter().map(|event| BASE64.encode(event)).collect::<Vec<_>>(),
        "collected": collected.iter().map(|event| BASE64.encode(event)).collect::<Vec<_>>(),
        "yielded_matches_collected": yielded == collected,
    })
}

// the provider spelling LiteLLM's LlmProviders enum carries
const fn provider_spelling(provider: GenAiProvider) -> &'static str {
    match provider {
        GenAiProvider::Gemini => "gemini",
        GenAiProvider::VertexAi => "vertex_ai",
    }
}

// the EndpointType member LiteLLM selects for each provider, by its enum name
const fn endpoint_name(provider: GenAiProvider) -> &'static str {
    match provider.endpoint_type() {
        EndpointType::VertexAi => "VERTEX_AI",
        EndpointType::Gemini => "GEMINI",
        EndpointType::Anthropic => "ANTHROPIC",
        EndpointType::OpenAi => "OPENAI",
        EndpointType::Tinyfish => "TINYFISH",
        EndpointType::Generic => "GENERIC",
        EndpointType::Decisions => "DECISIONS",
    }
}

fn stream_metadata(input: &Input) -> GenAiStreamMetadata {
    GenAiStreamMetadata {
        model: input.model.clone(),
        provider: GenAiProvider::from_spelling(&input.provider),
        request_body: IndexMap::new(),
        hidden_params: HiddenParams::new(),
    }
}

// each line response.iter_lines() / aiter_lines() yields comes with its trailing newline
fn line_bytes(lines: &[String]) -> Vec<u8> {
    lines
        .iter()
        .flat_map(|line| line.bytes().chain(std::iter::once(b'\n')))
        .collect()
}

struct WallClock;

impl SuccessClock for WallClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}

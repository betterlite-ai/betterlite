use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_character_id, decode_video_id_with_provider, encode_character_id,
    encode_video_id_with_provider, rewrite_character_ids,
};
use betterlite_openai::openai_video_identifiers::multipart::{
    MediaBody, MultipartPart, add_image_to_files, video_file_tuple,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::helpers::{Helper, HelperResult};
use fixtures::{Fixture, prepared};

mod fixtures;
mod tests;

const IMAGE_FIELD: &str = "input_reference";
const VIDEO_FIELD: &str = "video";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Encode {
    #[serde(alias = "video_id", alias = "character_id")]
    id: String,
    provider: String,
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Decode {
    encoded: String,
    capture_debug: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Call {
    EncodeVideoId(Encode),
    DecodeVideoId(Decode),
    ExtractOriginalVideoId(Decode),
    EncodeCharacterId(Encode),
    DecodeCharacterId(Decode),
    ExtractOriginalCharacterId(Decode),
    RewriteCharacters { request: Map<String, Value> },
    AddImageToFiles { fixture: Fixture },
    VideoFileTuple { fixture: Fixture },
}

fn render_part(part: &MultipartPart<'_>) -> Value {
    let (kind, body) = match &part.body {
        MediaBody::Bytes(bytes) => ("bytes", Value::String(STANDARD.encode(bytes))),
        MediaBody::Text(text) => ("text", Value::String((*text).to_owned())),
        MediaBody::Stream { .. } => ("stream", Value::Null),
    };
    json!({
        "field": part.field.as_str(),
        "filename": part.filename.as_str(),
        "content_type": part.content_type,
        "body_kind": kind,
        "body": body,
        "original": true,
    })
}

fn image_files(fixture: &Fixture) -> Value {
    let mut state = prepared(fixture);
    let mut files: Vec<MultipartPart<'_>> = Vec::new();
    let body = state.media_body();
    add_image_to_files(&mut files, IMAGE_FIELD, body);
    let parts: Vec<Value> = files.iter().map(render_part).collect();
    drop(files);
    json!({"parts": parts, "position": state.position()})
}

fn video_files(fixture: &Fixture) -> Value {
    let mut state = prepared(fixture);
    let body = state.media_body();
    let part = video_file_tuple(body, VIDEO_FIELD);
    let rendered = render_part(&part);
    drop(part);
    json!({"parts": [rendered], "position": state.position()})
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn observed<T, F: FnOnce() -> T>(logs: &Logs, capture: bool, work: F) -> (T, bool) {
    if !capture {
        return (work(), false);
    }
    logs.0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, work);
    let logged = !logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty();
    (value, logged)
}

fn decoded_video(logs: &Logs, decode: &Decode) -> Value {
    let (decoded, logged) = observed(logs, decode.capture_debug, || {
        decode_video_id_with_provider(&decode.encoded)
    });
    json!({
        "custom_llm_provider": decoded.custom_llm_provider,
        "model_id": decoded.model_id,
        "video_id": decoded.video_id,
        "logged": logged,
    })
}

fn decoded_character(logs: &Logs, decode: &Decode) -> Value {
    let (decoded, logged) = observed(logs, decode.capture_debug, || {
        decode_character_id(&decode.encoded)
    });
    json!({
        "custom_llm_provider": decoded.custom_llm_provider,
        "model_id": decoded.model_id,
        "character_id": decoded.character_id,
        "logged": logged,
    })
}

fn extracted<F: FnOnce(&str) -> String>(logs: &Logs, decode: &Decode, extract: F) -> Value {
    let (id, logged) = observed(logs, decode.capture_debug, || extract(&decode.encoded));
    json!({"id": id, "logged": logged})
}

fn run_call(call: &Call, logs: &Logs) -> Value {
    match call {
        Call::EncodeVideoId(encode) => json!(encode_video_id_with_provider(
            &encode.id,
            &encode.provider,
            encode.model_id.as_deref()
        )),
        Call::DecodeVideoId(decode) => decoded_video(logs, decode),
        Call::ExtractOriginalVideoId(decode) => extracted(logs, decode, |encoded| {
            decode_video_id_with_provider(encoded).video_id
        }),
        Call::EncodeCharacterId(encode) => json!(encode_character_id(
            &encode.id,
            &encode.provider,
            encode.model_id.as_deref()
        )),
        Call::DecodeCharacterId(decode) => decoded_character(logs, decode),
        Call::ExtractOriginalCharacterId(decode) => extracted(logs, decode, |encoded| {
            decode_character_id(encoded).character_id
        }),
        Call::RewriteCharacters { request } => {
            let mut rewritten = request.clone();
            rewrite_character_ids(&mut rewritten);
            Value::Object(rewritten)
        }
        Call::AddImageToFiles { fixture } => image_files(fixture),
        Call::VideoFileTuple { fixture } => video_files(fixture),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let logs = Logs::default();
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call, &logs));
        }
        Ok(Value::Array(results))
    }
}

#![cfg(test)]

use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_compression::compress::CompressionSkippedReason;
use betterlite_config::sections::compression_interception_state::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::compression_interception::cache::Clock;
use crate::api::compression_interception::engine::{
    CompressionCall, CompressionEngine, CompressionOutput,
};
use crate::api::compression_interception::hook::{
    CompressionHookError, CompressionInterception, HookResult,
};
use crate::routes::CallType;

pub(super) const NOW: f64 = 1000.0;

#[derive(Debug, Error)]
#[error("the scripted engine failed")]
pub(super) struct ScriptError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedCall {
    pub model: String,
    pub call_type: CallType,
    pub messages: Vec<Json>,
    pub compression_trigger: usize,
    pub compression_target: Option<usize>,
    pub embedding_model: Option<String>,
    pub embedding_model_params: Option<IndexMap<String, Json>>,
}

pub(super) struct ScriptedEngine {
    outputs: Mutex<VecDeque<Result<CompressionOutput, ScriptError>>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedEngine {
    pub(super) fn new(outputs: Vec<Result<CompressionOutput, ScriptError>>) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(VecDeque::from(outputs)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CompressionEngine for ScriptedEngine {
    type Failure = ScriptError;

    async fn compress(&self, call: &CompressionCall<'_>) -> Result<CompressionOutput, ScriptError> {
        let recorded = RecordedCall {
            model: call.model.to_owned(),
            call_type: call.call_type,
            messages: call.messages.to_vec(),
            compression_trigger: call.compression_trigger,
            compression_target: call.compression_target,
            embedding_model: call.embedding_model.map(str::to_owned),
            embedding_model_params: call.embedding_model_params.cloned(),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        let mut outputs = self.outputs.lock().unwrap_or_else(PoisonError::into_inner);
        outputs.pop_front().unwrap_or(Err(ScriptError))
    }
}

pub(super) fn disabled() -> Settings {
    Settings {
        enabled: false,
        compression_trigger: 200_000,
        compression_target: None,
        embedding_model: None,
        embedding_model_params: None,
    }
}

pub(super) fn compressed_output(cache: IndexMap<String, String>) -> CompressionOutput {
    output(
        vec![object(vec![
            ("role", text("user")),
            ("content", text("compressed")),
        ])],
        Vec::new(),
        number(100),
        number(40),
        cache,
    )
}

pub(super) fn fixed_clock(now: f64) -> Clock {
    Arc::new(move || now)
}

pub(super) fn manual_clock(start: f64) -> (Clock, Arc<Mutex<f64>>) {
    let value = Arc::new(Mutex::new(start));
    let handle = Arc::clone(&value);
    let clock: Clock = Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner));
    (clock, value)
}

pub(super) struct Harness {
    pub interception: CompressionInterception<ScriptedEngine>,
    pub engine: Arc<ScriptedEngine>,
}

pub(super) fn harness(
    settings: Settings,
    clock: Clock,
    outputs: Vec<Result<CompressionOutput, ScriptError>>,
) -> Harness {
    let engine = ScriptedEngine::new(outputs);
    Harness {
        interception: CompressionInterception::with_clock(settings, Arc::clone(&engine), clock),
        engine,
    }
}

pub(super) fn output(
    messages: Vec<Json>,
    tools: Vec<Json>,
    original_tokens: Json,
    compressed_tokens: Json,
    cache: IndexMap<String, String>,
) -> CompressionOutput {
    CompressionOutput {
        messages,
        tools,
        original_tokens,
        compressed_tokens,
        cache,
        skipped_reason: None,
    }
}

pub(super) fn skipped_output(reason: CompressionSkippedReason, tokens: i64) -> CompressionOutput {
    CompressionOutput {
        messages: Vec::new(),
        tools: Vec::new(),
        original_tokens: number(tokens),
        compressed_tokens: number(tokens),
        cache: IndexMap::new(),
        skipped_reason: Some(reason),
    }
}

pub(super) fn cache(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn list(items: Vec<Json>) -> Json {
    Json::List(items)
}

pub(super) fn object(pairs: Vec<(&str, Json)>) -> Json {
    Json::Object(index(pairs))
}

pub(super) fn index(pairs: Vec<(&str, Json)>) -> IndexMap<String, Json> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn request() -> IndexMap<String, Json> {
    index(vec![
        ("model", text("claude-sonnet-4")),
        (
            "messages",
            list(vec![object(vec![
                ("role", text("user")),
                ("content", text("hi")),
            ])]),
        ),
    ])
}

pub(super) fn request_with_tools(tools: Json) -> IndexMap<String, Json> {
    let mut body = request();
    body.insert("tools".to_owned(), tools);
    body
}

pub(super) async fn apply(
    harness: &Harness,
    body: &mut IndexMap<String, Json>,
    call_type: Option<CallType>,
) -> Result<HookResult, CompressionHookError<ScriptError>> {
    harness
        .interception
        .pre_call_deployment_hook(body, call_type)
        .await
}

pub(super) async fn applied(harness: &Harness, body: &mut IndexMap<String, Json>) {
    let result = apply(harness, body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Applied);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Recorded {
    pub level: Level,
    pub kind: String,
    pub reason: Option<String>,
    pub call_id: Option<String>,
    pub original: Option<String>,
    pub compressed: Option<String>,
    pub cached_keys: Option<u64>,
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<Recorded>>>);

impl Captured {
    pub(super) fn events(&self) -> Vec<Recorded> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Fields {
    kind: Option<String>,
    reason: Option<String>,
    call_id: Option<String>,
    original: Option<String>,
    compressed: Option<String>,
    cached_keys: Option<u64>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "kind" => self.kind = Some(value.to_owned()),
            "reason" => self.reason = Some(value.to_owned()),
            "call_id" => self.call_id = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        match field.name() {
            "original" => self.original = Some(format!("{value:?}")),
            "compressed" => self.compressed = Some(format!("{value:?}")),
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "cached_keys" {
            self.cached_keys = Some(value);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if !event
            .metadata()
            .target()
            .starts_with("betterlite_core::api::compression_interception")
        {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let Some(kind) = fields.kind else {
            return;
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                level: *event.metadata().level(),
                kind,
                reason: fields.reason,
                call_id: fields.call_id,
                original: fields.original,
                compressed: fields.compressed,
                cached_keys: fields.cached_keys,
            });
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let guard = set_default(registry().with(captured.clone()));
    (captured, guard)
}

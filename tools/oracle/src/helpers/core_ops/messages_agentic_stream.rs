mod fakes;
mod lines;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_agentic_stream::HoldBackPolicy;
use betterlite_core::api::messages_agentic_stream::MessagesAgenticStreamInput;
use betterlite_core::api::messages_agentic_stream::stream::MessagesAgenticStream;
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_core::meta_muse_realtime::request::Sleep;
use betterlite_models::decisions::Json;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tracing::subscriber;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_helper_api::helper::HelperResult;
use fakes::ScriptedInner;
use fakes::ScriptedInterceptor;
use fakes::Shared;
use lines::Lines;
use tokio::runtime::Builder;

pub(crate) type Stream = MessagesAgenticStream<ScriptedInner, ScriptedInterceptor>;

const CALL_ID: &str = "call-1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    chunks: Vec<String>,
    hold_back: bool,
    server_fulfilled_tool_names: Vec<String>,
    ping_interval_seconds: f64,
    reply: Reply,
    steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    kind: ReplyKind,
    chunks: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyKind {
    Absent,
    Stream,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Read,
    ReadAll,
    ReleaseDrain,
    ReleaseHook,
    Close,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        let lines = Lines::default();
        let guard = subscriber::set_default(registry().with(lines.clone()));
        let outcome = runtime.block_on(drive(input));
        drop(guard);
        let (results, buffered, shared) = outcome?;
        Ok(answer(&shared, &results, &buffered, &lines))
    }
}

async fn drive(input: &Input) -> Result<(Vec<Value>, Vec<Value>, Shared), Box<HelperError>> {
    let shared = Shared {
        drain_gate: Arc::new(AtomicBool::new(false)),
        hook_gate: Arc::new(AtomicBool::new(false)),
        effects: Arc::new(Mutex::new(Vec::new())),
        hook_calls: Arc::new(Mutex::new(Vec::new())),
    };
    let mut stream = build_stream(input, &shared);
    let mut results = Vec::new();
    let mut buffered = Vec::new();
    for step in &input.steps {
        match step {
            Step::Read => {
                read_one(&mut stream, &mut results, &mut buffered).await?;
            }
            Step::ReadAll => {
                let mut more = true;
                while more {
                    more = read_one(&mut stream, &mut results, &mut buffered).await?;
                }
            }
            Step::ReleaseDrain => shared.drain_gate.store(true, Ordering::SeqCst),
            Step::ReleaseHook => shared.hook_gate.store(true, Ordering::SeqCst),
            Step::Close => close(&mut stream).await?,
        }
    }
    Ok((results, buffered, shared))
}

fn build_stream(input: &Input, shared: &Shared) -> Stream {
    let policy = HoldBackPolicy {
        server_fulfilled_tool_names: input.server_fulfilled_tool_names.iter().cloned().collect(),
        ping_interval_seconds: input.ping_interval_seconds,
    };
    let agentic_input = MessagesAgenticStreamInput {
        model: input.model.clone(),
        messages: Vec::new(),
        optional_request_params: IndexMap::new(),
        logging: AgenticLogging {
            call_id: Some(CALL_ID.to_owned()),
            agentic_loop_params: None,
        },
        custom_llm_provider: "anthropic".to_owned(),
        kwargs: IndexMap::new(),
        hold_back: input.hold_back.then_some(policy),
    };
    let inner = ScriptedInner {
        frames: input
            .chunks
            .iter()
            .map(|chunk| chunk.as_bytes().to_vec())
            .collect(),
        gate: Arc::clone(&shared.drain_gate),
        effects: Arc::clone(&shared.effects),
    };
    let interceptor = ScriptedInterceptor {
        reply: Mutex::new(Some(input.reply.clone())),
        gate: Arc::clone(&shared.hook_gate),
        hook_calls: Arc::clone(&shared.hook_calls),
        effects: Arc::clone(&shared.effects),
    };
    let sleep: Sleep = Arc::new(|_seconds: f64| -> BoxFuture<'static, ()> { Box::pin(async {}) });
    MessagesAgenticStream::new(
        inner,
        interceptor,
        agentic_input,
        sleep,
        ApiLimits::default(),
    )
}

async fn read_one(
    stream: &mut Stream,
    results: &mut Vec<Value>,
    buffered: &mut Vec<Value>,
) -> Result<bool, Box<HelperError>> {
    let chunk = stream
        .next_chunk()
        .await
        .map_err(|error| failure("RuntimeError", error.to_string()))?;
    let ended = chunk.is_none();
    results.push(frame_value(chunk)?);
    buffered.push(Value::Bool(
        stream.buffered_provider_output() == Some(Json::Bool(true)),
    ));
    Ok(!ended)
}

fn frame_value(chunk: Option<Vec<u8>>) -> Result<Value, Box<HelperError>> {
    let Some(bytes) = chunk else {
        return Ok(Value::Null);
    };
    let text = String::from_utf8(bytes)
        .map_err(|error| failure("UnicodeDecodeError", error.to_string()))?;
    Ok(Value::String(text))
}

async fn close(stream: &mut Stream) -> Result<(), Box<HelperError>> {
    let Some(closed) = stream.aclose().await else {
        return Err(failure(
            "RuntimeError",
            "the agentic stream reports no aclose".to_owned(),
        ));
    };
    closed.map_err(|error| failure("RuntimeError", error.to_string()))
}

fn answer(shared: &Shared, results: &[Value], buffered: &[Value], lines: &Lines) -> Value {
    let hook_calls = shared
        .hook_calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let effects = shared
        .effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let logs = lines
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!({
        "results": results,
        "buffered": buffered,
        "hook_calls": hook_calls,
        "effects": effects,
        "logs": logs,
    })
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

use std::collections::BTreeMap;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_bedrock::agentcore_stream::transport::{HandlerKind, StreamHandler};
use betterlite_bedrock::bedrock_http_stream::StreamDeps;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::invoke_stream::{
    InvokeStreamCall, WrappedInvokeStream, stream_transport, sync_client,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use betterlite_oracle_helper_api::error::HelperError;

use super::fixture::Fixture;
use super::support::{
    Quiet, World, failure, header_map, logging_state, pre_post_env, stream_deps, wrapper_of,
};
use super::types::{Client, Input, Run, Wrapper};
use super::views::{arguments_view, config_view, log_view, wrapper_view};

pub(super) fn execute(input: &Input, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let cases = match input {
        Input::TransportArguments { runs } => runs
            .iter()
            .map(transport_case)
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
        Input::WrapperIdentity { runs } => runs
            .iter()
            .map(|run| identity_case(run, runtime))
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
    };
    Ok(json!({ "cases": cases }))
}

// The wrapper's own work is one pure call here: serialize, switch the fake stream, pick the provider, keep the chunk size.
fn transport_case(run: &Run) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    let deps = case_deps(run, &redactor)?;
    let call = call_of(run)?;
    let transport = stream_transport(&call, run.wrapper == Wrapper::Sync);
    Ok(json!({
        "arguments": arguments_view(&transport),
        "client": client_label(run.wrapper, &deps),
        "config": config_view(),
    }))
}

// A real call through B691 and B004: the POST, the log entry and the wrapper's identity around the stream.
fn identity_case(run: &Run, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let kind = run.identity_kind().map_err(failure)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    let body = hex::decode(&run.response_body).map_err(failure)?;
    let supplied = Fixture::make(kind, &run.response_headers, body, run.response_fails)?;
    let shared: Arc<dyn StreamHandler> = supplied.clone();
    let default = handler(wrapper_kind(run.wrapper), &BTreeMap::new())?;
    let deps = stream_deps(Some(shared), default, &redactor);
    let call = call_of(run)?;
    let transport = stream_transport(&call, run.wrapper == Wrapper::Sync);
    let recorded = Mutex::new(Vec::new());
    let sink = Quiet;
    let logger = |details: &Json| -> Result<(), PrePostError> {
        recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    };
    let env = pre_post_env(&redactor, &sink, &logger)?;
    let mut state = logging_state(&redactor)?;
    let mut world = World::default();
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let mut wrapped = wrapper_of(runtime, &transport, &deps, &mut world, &mut log)?;
    let logs = mem::take(&mut *recorded.lock().unwrap_or_else(PoisonError::into_inner));
    let (text, failed) = drain(runtime, &mut wrapped);
    Ok(json!({
        "wrapper": wrapper_view(&wrapped),
        "config": config_view(),
        "log": logs.first().map_or(Value::Null, log_view),
        "stream": {"text": text, "failed": failed},
        "post": supplied.first_post(),
    }))
}

// The callers' read loop: the text each chunk carries, and whether the stream ended in an error.
fn drain(runtime: &Runtime, wrapped: &mut WrappedInvokeStream<'_>) -> (String, bool) {
    let mut text = String::new();
    loop {
        match runtime.block_on(wrapped.completion_stream.next()) {
            Ok(Some(chunk)) => {
                if let Some(content) = chunk
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.content.clone())
                {
                    text.push_str(&content);
                }
            }
            Ok(None) => return (text, false),
            Err(_) => return (text, true),
        }
    }
}

fn call_of(run: &Run) -> Result<InvokeStreamCall<'_>, Box<HelperError>> {
    Ok(InvokeStreamCall {
        model: &run.model,
        api_base: &run.api_base,
        headers: header_map(&run.headers)?,
        data: &run.data,
        messages: &run.messages,
        json_mode: run.json_mode,
        signed_json_body: run.signed_json_body.as_deref().map(str::as_bytes),
        control: ControlOptions::new(run.stream_chunk_size).map_err(failure)?,
    })
}

fn case_deps<'deps>(
    run: &Run,
    redactor: &'deps Redactor,
) -> Result<StreamDeps<'deps>, Box<HelperError>> {
    let supplied = match run.client {
        Client::None => None,
        Client::Async => Some(handler(HandlerKind::Async, &run.response_headers)?),
        Client::Sync => Some(handler(HandlerKind::Sync, &run.response_headers)?),
    };
    let default = handler(wrapper_kind(run.wrapper), &BTreeMap::new())?;
    Ok(stream_deps(supplied, default, redactor))
}

// Async keeps whatever the caller supplied; sync keeps it only when it is the sync client B173 expects.
fn client_label(wrapper: Wrapper, deps: &StreamDeps<'_>) -> &'static str {
    match wrapper {
        Wrapper::Async => match deps.supplied {
            Some(_) => "supplied",
            None => "none",
        },
        Wrapper::Sync => match &deps.supplied {
            Some(supplied) if Arc::ptr_eq(supplied, &sync_client(deps)) => "supplied",
            Some(_) | None => "module",
        },
    }
}

fn handler(
    kind: HandlerKind,
    headers: &BTreeMap<String, String>,
) -> Result<Arc<dyn StreamHandler>, Box<HelperError>> {
    let fixture: Arc<Fixture> = Fixture::make(kind, headers, Vec::new(), false)?;
    let shared: Arc<dyn StreamHandler> = fixture;
    Ok(shared)
}

const fn wrapper_kind(wrapper: Wrapper) -> HandlerKind {
    match wrapper {
        Wrapper::Async => HandlerKind::Async,
        Wrapper::Sync => HandlerKind::Sync,
    }
}

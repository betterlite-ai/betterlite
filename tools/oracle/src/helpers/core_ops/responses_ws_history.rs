mod support;
mod views;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_core::api::responses_ws_history::ManagedResponsesWebSocket;
use betterlite_core::api::responses_ws_history::call_kwargs::update_proxy_request;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::support::{Socket, frame_count, helper_error, sent_frames};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Setup {
    model: String,
    custom_llm_provider: Option<String>,
    api_key: Option<String>,
    api_base: Option<String>,
    timeout: Option<f64>,
    first_message: Option<String>,
    litellm_metadata: Option<IndexMap<String, Json>>,
    kwargs: IndexMap<String, Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InjectRow {
    kwargs: IndexMap<String, Json>,
    model: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorRow {
    message: String,
    error_type: Option<String>,
    fail: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParseRow {
    raw: String,
    fail: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreRow {
    id: String,
    messages: Vec<IndexMap<String, Json>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryCase {
    store: Vec<StoreRow>,
    first: String,
    append: IndexMap<String, Json>,
    missing: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyRow {
    previous_response_id: Option<String>,
    current: Vec<IndexMap<String, Json>>,
    prior: Vec<IndexMap<String, Json>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    setup: Setup,
    resolve: Vec<Option<String>>,
    inject: Vec<InjectRow>,
    proxy: Vec<IndexMap<String, Json>>,
    serialize: Vec<String>,
    errors: Vec<ErrorRow>,
    parse: Vec<ParseRow>,
    warmup: Vec<IndexMap<String, Json>>,
    warmup_ids: Vec<Option<String>>,
    inputs: Vec<Json>,
    completions: Vec<IndexMap<String, Json>>,
    history: HistoryCase,
    base_kwargs: Vec<IndexMap<String, Json>>,
    apply: Vec<ApplyRow>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|source| helper_error("RuntimeError", None, source.to_string()))?;
        let socket = Socket {
            frames: Arc::new(Mutex::new(Vec::new())),
            fail_next: Arc::new(AtomicBool::new(false)),
        };
        let mut handler = support::handler(&input.setup, socket.clone());
        runtime.block_on(answers(&mut handler, &socket, input))
    }
}

async fn answers(
    handler: &mut ManagedResponsesWebSocket<Socket>,
    socket: &Socket,
    input: &Input,
) -> HelperResult {
    let constructor = constructor_view(handler);
    let resolve = resolve_view(handler, &input.resolve);
    let inject = inject_view(handler, &input.inject)?;
    let proxy = proxy_view(handler, &input.proxy)?;
    let serialize = views::serialize_view(&input.serialize)?;
    let errors = errors_view(handler, socket, &input.errors).await?;
    let parse = parse_view(handler, socket, &input.parse).await?;
    let warmup = views::warmup_view(handler, socket, &input.warmup, &input.warmup_ids).await?;
    let inputs = views::inputs_view(&input.inputs);
    let completions = views::completions_view(&input.completions)?;
    let history = views::history_view(handler, &input.history);
    let base_kwargs = views::base_kwargs_view(&input.base_kwargs);
    let apply = views::apply_view(&input.apply);
    Ok(json!({
        "constructor": constructor,
        "resolve": resolve,
        "inject": inject,
        "proxy": proxy,
        "serialize": serialize,
        "errors": errors,
        "parse": parse,
        "warmup": warmup,
        "inputs": inputs,
        "completions": completions,
        "history": history,
        "base_kwargs": base_kwargs,
        "apply": apply,
    }))
}

fn constructor_view(handler: &ManagedResponsesWebSocket<Socket>) -> Value {
    let api_key = match &handler.api_key {
        Some(key) => Value::String(key.expose_secret().to_owned()),
        None => Value::Null,
    };
    json!({
        "model": handler.model,
        "model_group": handler.model_group,
        "connection_provider": handler.connection_provider,
        "litellm_metadata": handler.litellm_metadata,
        "api_key": api_key,
        "api_base": handler.api_base,
        "timeout": handler.timeout,
        "custom_llm_provider": handler.custom_llm_provider,
        "first_message": handler.first_message,
        "quota_callbacks": handler.quota_callbacks.len(),
        "extra_kwargs": handler.extra_kwargs,
        "history_empty": handler.get_history_messages("resp_none").is_empty(),
    })
}

fn resolve_view(handler: &ManagedResponsesWebSocket<Socket>, models: &[Option<String>]) -> Value {
    let mut answers = Vec::with_capacity(models.len());
    for model in models {
        answers.push(json!({
            "model": model,
            "provider": handler.resolve_provider(model.as_deref()),
            "same": handler.same_provider(model.as_deref()),
        }));
    }
    Value::Array(answers)
}

fn inject_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    rows: &[InjectRow],
) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        let mut kwargs = row.kwargs.clone();
        handler
            .inject_credentials(&mut kwargs, row.model.as_deref())
            .map_err(|error| helper_error("TypeError", None, error.to_string()))?;
        answers.push(json!(kwargs));
    }
    Ok(Value::Array(answers))
}

fn proxy_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    rows: &[IndexMap<String, Json>],
) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        let mut kwargs = row.clone();
        update_proxy_request(&mut kwargs, &handler.model)
            .map_err(|error| helper_error("TypeError", None, error.to_string()))?;
        answers.push(json!(kwargs));
    }
    Ok(Value::Array(answers))
}

async fn errors_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    socket: &Socket,
    rows: &[ErrorRow],
) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        let start = frame_count(socket);
        socket.fail_next.store(row.fail, Ordering::SeqCst);
        handler
            .send_error(&row.message, row.error_type.as_deref())
            .await;
        answers.push(json!(sent_frames(socket, start)?));
    }
    Ok(Value::Array(answers))
}

async fn parse_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    socket: &Socket,
    rows: &[ParseRow],
) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        let start = frame_count(socket);
        socket.fail_next.store(row.fail, Ordering::SeqCst);
        let message = handler
            .parse_message(&row.raw)
            .await
            .map_err(|error| helper_error("AttributeError", None, error.to_string()))?;
        let frames = sent_frames(socket, start)?;
        match message {
            None => answers.push(json!({"kind": "ignored", "message": null, "frames": frames})),
            Some(fields) => answers.push(json!({
                "kind": "message",
                "message": Json::Object(fields),
                "frames": frames,
            })),
        }
    }
    Ok(Value::Array(answers))
}

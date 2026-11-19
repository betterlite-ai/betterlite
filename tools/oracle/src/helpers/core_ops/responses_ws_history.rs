mod support;
mod views;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use betterlite_core::api::responses_ws_history::ManagedResponsesWebSocket;
use betterlite_core::api::responses_ws_history::call_kwargs::{
    inject_credentials, update_proxy_request,
};
use betterlite_core::api::responses_ws_history::history::get_history_messages;
use betterlite_core::api::responses_ws_history::provider::{resolve_provider, same_provider};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::support::{Socket, build_handler, helper_error};
use self::views::{frames_view, history_view, messages_view, warmup_view};

#[derive(Deserialize)]
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
struct CredentialRow {
    model: Option<String>,
    kwargs: IndexMap<String, Json>,
}

#[derive(Deserialize)]
struct FrameRow {
    kind: String,
    payload: String,
    error_type: Option<String>,
    fail: bool,
}

#[derive(Deserialize)]
struct StoreRow {
    id: String,
    messages: Vec<IndexMap<String, Json>>,
}

#[derive(Deserialize)]
struct ApplyRow {
    previous_response_id: Option<String>,
    current: Vec<IndexMap<String, Json>>,
    prior: Vec<IndexMap<String, Json>>,
}

#[derive(Deserialize)]
struct Case {
    inputs: Vec<Json>,
    completions: Vec<IndexMap<String, Json>>,
    store: Vec<StoreRow>,
    first: String,
    append: IndexMap<String, Json>,
    missing: String,
    applied: Vec<ApplyRow>,
    base_kwargs: Vec<IndexMap<String, Json>>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    setup: Setup,
    credentials: Vec<CredentialRow>,
    proxy: Vec<IndexMap<String, Json>>,
    frames: Vec<FrameRow>,
    warmup: Vec<IndexMap<String, Json>>,
    warmup_ids: Vec<Option<String>>,
    case: Case,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|source| helper_error("RuntimeError", source.to_string()))?;
        let socket = Socket {
            frames: Arc::new(Mutex::new(Vec::new())),
            fail_next: Arc::new(AtomicBool::new(false)),
        };
        let mut handler = build_handler(&input.setup, socket.clone());
        runtime.block_on(answers(&mut handler, &socket, input))
    }
}

async fn answers(
    handler: &mut ManagedResponsesWebSocket<Socket>,
    socket: &Socket,
    input: &Input,
) -> HelperResult {
    let mut proxy = Vec::with_capacity(input.proxy.len());
    for row in &input.proxy {
        let mut kwargs = row.clone();
        update_proxy_request(&mut kwargs, &handler.model)
            .map_err(|error| helper_error("TypeError", error.to_string()))?;
        proxy.push(json!(kwargs));
    }
    Ok(json!({
        "constructor": constructor_view(handler),
        "credentials": credentials_view(handler, &input.credentials)?,
        "proxy": proxy,
        "frames": frames_view(handler, socket, &input.frames).await?,
        "warmup": warmup_view(handler, socket, &input.warmup, &input.warmup_ids).await?,
        "messages": messages_view(&input.case)?,
        "history": history_view(handler, &input.case),
    }))
}

fn constructor_view(handler: &ManagedResponsesWebSocket<Socket>) -> Value {
    json!({
        "model": handler.model,
        "model_group": handler.model_group,
        "connection_provider": handler.connection_provider,
        "litellm_metadata": handler.litellm_metadata,
        "api_key": handler.api_key.as_ref().map(|key| key.expose_secret().to_owned()),
        "api_base": handler.api_base,
        "timeout": handler.timeout,
        "custom_llm_provider": handler.custom_llm_provider,
        "first_message": handler.first_message,
        "quota_callbacks": handler.quota_callbacks.len(),
        "extra_kwargs": handler.extra_kwargs,
        "history_empty": get_history_messages(handler, "resp_none").is_empty(),
    })
}

fn credentials_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    rows: &[CredentialRow],
) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        let mut kwargs = row.kwargs.clone();
        inject_credentials(handler, &mut kwargs, row.model.as_deref())
            .map_err(|error| helper_error("TypeError", error.to_string()))?;
        answers.push(json!({
            "model": row.model,
            "provider": resolve_provider(handler, row.model.as_deref()),
            "same": same_provider(handler, row.model.as_deref()),
            "kwargs": kwargs,
        }));
    }
    Ok(Value::Array(answers))
}

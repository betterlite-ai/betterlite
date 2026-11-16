use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::compresr_writeback::apply_compression;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::task::JoinSet;

use super::{Config, build};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
pub(crate) struct Apply {
    config: Config,
    reply: Reply,
    inputs: GuardrailApiInputs,
    headers: Option<BTreeMap<String, String>>,
    caller: Option<Caller>,
}

#[derive(Clone, Deserialize)]
struct Reply {
    status: u16,
    body: String,
}

#[derive(Deserialize)]
struct Caller {
    scope: String,
    call_id: String,
}

type Seen = Arc<Mutex<Vec<Value>>>;

async fn record(
    State((reply, seen)): State<(Reply, Seen)>,
    request: Request,
) -> (StatusCode, String) {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_REQUEST_BYTES).await.unwrap_or_default();
    let api_key = parts
        .headers
        .get("x-api-key")
        .and_then(|key| key.to_str().ok());
    let sent = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let entry =
        json!({"path": parts.uri.path(), "api_key": api_key.unwrap_or_default(), "body": sent});
    seen.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(entry);
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, reply.body)
}

// What the proxy leaves on the logging object once a virtual key is authenticated
fn log_target(caller: &Caller) -> Result<GuardrailLogTarget, Box<HelperError>> {
    let params = json!({"metadata": {"user_api_key_auth": {"api_key": caller.scope}}});
    let details = json!({"litellm_call_id": caller.call_id});
    Ok(GuardrailLogTarget {
        litellm_params: Some(serde_json::from_value(params).or_value_error()?),
        model_call_params: Some(serde_json::from_value(details).or_value_error()?),
    })
}

pub(super) fn run(call: &Apply) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let listener = runtime
        .block_on(TcpListener::bind("127.0.0.1:0"))
        .or_value_error()?;
    let api_base = format!("http://{}", listener.local_addr().or_value_error()?);
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let state = (call.reply.clone(), Arc::clone(&seen));
    let router = Router::new().fallback(record).with_state(state);
    let mut server = JoinSet::new();
    // The server stops when the helper returns and the set aborts it
    server.spawn_on(
        async move {
            let _served = axum::serve(listener, router).await;
        },
        runtime.handle(),
    );
    let guardrail = build(&call.config, Some(&api_base), &BTreeMap::new()).or_value_error()?;
    let mut data: BTreeMap<String, GuardrailValue> = match &call.headers {
        Some(headers) => {
            serde_json::from_value(json!({"proxy_server_request": {"headers": headers}}))
                .or_value_error()?
        }
        None => BTreeMap::new(),
    };
    let mut target = call.caller.as_ref().map(log_target).transpose()?;
    let output = runtime
        .block_on(apply_compression(
            &guardrail,
            call.inputs.clone(),
            &mut data,
            GuardrailInputType::Request,
            target.as_mut(),
        ))
        .or_value_error()?;
    let requests = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(json!({"inputs": serde_json::to_value(output).or_value_error()?, "requests": requests}))
}

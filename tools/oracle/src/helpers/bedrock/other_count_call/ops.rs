use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::count_call::counter::{
    BedrockTokenCounter, CountTokensCall, TokenCountResponse, TokenCounter as _,
};
use betterlite_bedrock::count_call::factory::BedrockModelInfo;
use betterlite_bedrock::count_call::handler::{BedrockCountTokensHandler, CountTokensEnvironment};
use betterlite_bedrock::count_call::reply::CountResponse;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tracing::Level;

use crate::helpers::{HelperError, OrHelperError as _};

use super::input::{Call, Environment, Handler};
use super::logs::Captured;
use super::script::{FixedHandler, FixtureCredentials, Script};

pub(super) fn run_call(call: &Call, factory: bool) -> Result<Value, Box<HelperError>> {
    let (captured, guard) = super::logs::capture();
    let resolved = Arc::new(AtomicUsize::new(0));
    let transport = Script::new(&call.reply);
    let observed = transport.clone();
    let selection = BedrockTokenCounter {
        handler: FixedHandler { result: None },
    };
    let selected = selection.should_use_token_counting_api(call.provider.as_deref());
    let request = CountTokensCall {
        model_to_use: &call.model,
        messages: call.messages.as_ref(),
        deployment: call.deployment.as_ref(),
        request_model: &call.request_model,
        tools: call.tools.as_ref(),
        system: call.system.as_ref(),
        now: epoch(call.now)?,
    };
    let outcome = runtime()?.block_on(count(call, factory, transport, &resolved, &request));
    drop(guard);
    Ok(json!({
        "selected": selected,
        "result": outcome.as_ref().map(result_json),
        "request": observed.observed()?,
        "debug": debug_events(&captured),
        "errors": error_events(&captured),
        "warnings": warning_events(&captured),
        "resolved": resolved.load(Ordering::Relaxed),
    }))
}

async fn count(
    call: &Call,
    factory: bool,
    transport: Script,
    resolved: &Arc<AtomicUsize>,
    request: &CountTokensCall<'_>,
) -> Option<TokenCountResponse> {
    let environment = environment_of(&call.environment);
    let credentials = FixtureCredentials {
        resolved: Arc::clone(resolved),
    };
    match &call.handler {
        Handler::None => {
            BedrockTokenCounter {
                handler: FixedHandler { result: None },
            }
            .count_tokens(request)
            .await
        }
        Handler::EmptyObject => {
            BedrockTokenCounter {
                handler: FixedHandler {
                    result: Some(CountResponse::new()),
                },
            }
            .count_tokens(request)
            .await
        }
        Handler::Real => {
            let counter = if factory {
                BedrockModelInfo { environment }.get_token_counter(transport, credentials)
            } else {
                BedrockTokenCounter {
                    handler: BedrockCountTokensHandler {
                        transport,
                        credentials,
                        environment,
                    },
                }
            };
            counter.count_tokens(request).await
        }
    }
}

fn result_json(result: &TokenCountResponse) -> Value {
    json!({
        "total_tokens": result.total_tokens,
        "request_model": result.request_model,
        "model_used": result.model_used,
        "tokenizer_type": result.tokenizer_type,
        "original_response": result.original_response,
        "error": result.error,
        "error_message": error_message_kind(result.error_message.as_deref(), result.status_code),
        "status_code": result.status_code,
    })
}

fn error_message_kind(text: Option<&str>, status: Option<u16>) -> Option<&str> {
    let message = text?;
    match status {
        Some(500) if message.starts_with("CountTokens processing error: ") => Some("processing"),
        Some(500) => Some("generic"),
        Some(_) | None => Some(message),
    }
}

fn debug_events(captured: &Captured) -> Vec<Value> {
    captured
        .entries(Level::DEBUG)
        .iter()
        .map(|entry| {
            json!({
                "subject": entry.subject,
                "value": entry.value.as_deref().map_or(Value::Null, |text| {
                    serde_json::from_str(text).unwrap_or(Value::Null)
                }),
            })
        })
        .collect()
}

fn error_events(captured: &Captured) -> Vec<Value> {
    captured
        .entries(Level::ERROR)
        .iter()
        .map(|entry| json!({"subject": entry.subject}))
        .collect()
}

fn warning_events(captured: &Captured) -> Vec<Value> {
    captured
        .entries(Level::WARN)
        .iter()
        .map(|entry| json!({"subject": entry.subject, "status": entry.status}))
        .collect()
}

fn environment_of(input: &Environment) -> CountTokensEnvironment {
    CountTokensEnvironment {
        region_name: input.region_name.clone(),
        region: input.region.clone(),
        session_region: input.session_region.clone(),
        runtime_endpoint: input.runtime_endpoint.clone(),
        bearer_token: input.bearer_token.clone().map(secrecy::SecretString::from),
    }
}

fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).or_value_error()?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or("now is out of range")
        .or_value_error()
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_value_error()
}

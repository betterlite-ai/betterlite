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
use tokio::runtime::Builder;

use crate::helpers::{HelperError, OrHelperError as _};

use super::input::{Call, Handler};
use super::logs::Captured;
use super::script::{FixedHandler, FixtureCredentials, Script};

pub(super) fn run_call(call: &Call, factory: bool) -> Result<Value, Box<HelperError>> {
    let (captured, guard) = super::logs::capture();
    let resolved = Arc::new(AtomicUsize::new(0));
    let transport = Script::new(&call.reply);
    let selected = BedrockTokenCounter {
        handler: FixedHandler { result: None },
    }
    .should_use_token_counting_api(call.provider.as_deref());
    let request = CountTokensCall {
        model_to_use: &call.model,
        messages: call.messages.as_ref(),
        deployment: call.deployment.as_ref(),
        request_model: &call.request_model,
        tools: call.tools.as_ref(),
        system: call.system.as_ref(),
        now: epoch(call.now)?,
    };
    let outcome = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?
        .block_on(count(call, factory, transport.clone(), &resolved, &request));
    drop(guard);
    Ok(json!({
        "selected": selected,
        "result": outcome.as_ref().map(result_json),
        "request": transport.observed(),
        "debug": events(&captured, "DEBUG", &["subject", "value"]),
        "errors": events(&captured, "ERROR", &["subject"]),
        "warnings": events(&captured, "WARN", &["subject", "status"]),
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
    let environment = environment(call);
    let credentials = FixtureCredentials {
        resolved: Arc::clone(resolved),
    };
    match &call.handler {
        Handler::None | Handler::EmptyObject => {
            let result = matches!(call.handler, Handler::EmptyObject).then(CountResponse::new);
            BedrockTokenCounter {
                handler: FixedHandler { result },
            }
            .count_tokens(request)
            .await
        }
        Handler::Real if factory => {
            BedrockModelInfo { environment }
                .get_token_counter(transport, credentials)
                .count_tokens(request)
                .await
        }
        Handler::Real => {
            BedrockTokenCounter {
                handler: BedrockCountTokensHandler {
                    transport,
                    credentials,
                    environment,
                },
            }
            .count_tokens(request)
            .await
        }
    }
}

fn environment(call: &Call) -> CountTokensEnvironment {
    CountTokensEnvironment {
        region_name: call.environment.region_name.clone(),
        region: call.environment.region.clone(),
        session_region: call.environment.session_region.clone(),
        runtime_endpoint: call.environment.runtime_endpoint.clone(),
        bearer_token: call
            .environment
            .bearer_token
            .clone()
            .map(secrecy::SecretString::from),
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

fn events(captured: &Captured, level: &str, keys: &[&str]) -> Vec<Value> {
    captured
        .fields(level)
        .iter()
        .map(|fields| {
            keys.iter()
                .map(|key| {
                    let field = fields.get(*key).cloned().unwrap_or(Value::Null);
                    let parsed = field
                        .as_str()
                        .filter(|_| *key == "value")
                        .and_then(|text| serde_json::from_str(text).ok());
                    (*key, parsed.unwrap_or(field))
                })
                .collect()
        })
        .collect()
}

fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).or_value_error()?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or("now is out of range")
        .or_value_error()
}

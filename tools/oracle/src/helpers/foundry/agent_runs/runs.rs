use std::sync::PoisonError;

use betterlite_foundry::agent_requests::prepare::prepare;
use betterlite_foundry::agent_requests::request::AgentCallRequest;
use betterlite_foundry::agent_results::reply::AgentReply;
use betterlite_foundry::agent_runs::flow::{CompletionCall, completion};
use betterlite_foundry::agent_runs::transport::{AgentRunMode, AgentRunTransport};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::fixtures::{
    CountingCounter, RecordingFactory, RecordingSleeper, ScriptedTransport, secret_headers,
};
use super::input::{Mode, RunCall};
use super::views::{reply_view, ssl_verify};

type Calls = Result<Value, Box<HelperError>>;

pub(super) fn runs(calls: &[RunCall], runtime: &Runtime) -> Calls {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(runtime.block_on(run_answer(call))?);
    }
    Ok(Value::Array(answers))
}

async fn run_answer(call: &RunCall) -> Calls {
    let key = SecretString::from(call.api_key.clone());
    let headers = secret_headers(&call.headers);
    let request = AgentCallRequest {
        model: &call.model,
        messages: &call.messages,
        optional_params: &call.optional_params,
        headers: &headers,
        api_key: Some(&key),
        api_base: Some(&call.api_base),
    };
    let prepared = prepare(&request).or_value_error()?;
    let ssl = ssl_verify(&call.litellm_params);
    let scripted = ScriptedTransport::new(&call.steps);
    let factory = RecordingFactory::new(&scripted);
    let sleeper = RecordingSleeper::default();
    let transport: &dyn AgentRunTransport = &scripted;
    let supplied = call.supplied.then_some(transport);
    let completion_call = CompletionCall {
        model: &call.model,
        messages: &call.messages,
        optional_params: &call.optional_params,
        prepared,
        ssl_verify: ssl.clone(),
        reply: AgentReply::default(),
        mode: match call.mode {
            Mode::Sync => AgentRunMode::Sync,
            Mode::Async => AgentRunMode::Async,
        },
        supplied,
    };
    let outcome = finished(&completion_call, &factory, &sleeper).await;
    answered(&factory, &scripted, &sleeper, call.supplied, ssl, outcome)
}

async fn finished(
    completion_call: &CompletionCall<'_>,
    factory: &RecordingFactory,
    sleeper: &RecordingSleeper,
) -> Value {
    match completion(completion_call, factory, sleeper, &CountingCounter).await {
        Ok(reply) => json!({"ok": true, "reply": reply_view(&reply)}),
        Err(error) => json!({
            "ok": false,
            "status": error.status(),
            "message": error.to_string(),
        }),
    }
}

fn answered(
    factory: &RecordingFactory,
    scripted: &ScriptedTransport,
    sleeper: &RecordingSleeper,
    supplied: bool,
    ssl: Json,
    outcome: Value,
) -> Calls {
    let client = if supplied {
        "supplied"
    } else {
        *factory.kind.lock().unwrap_or_else(PoisonError::into_inner)
    };
    let answered_ssl = if supplied { Json::Null } else { ssl };
    Ok(json!({
        "client": client,
        "ssl_verify": answered_ssl,
        "requests": scripted.requests(),
        "sleeps": sleeper.values(),
        "outcome": outcome,
    }))
}

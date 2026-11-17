use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_foundry::agent_results::reply::AgentReply;
use betterlite_foundry::agent_results::request::StreamRequest;
use betterlite_foundry::agent_runs::dispatch::{
    DispatchCall as FoundryDispatchCall, Route, dispatch,
};
use betterlite_foundry::agent_runs::error::AgentRunError;
use betterlite_foundry::agent_runs::flow::{CompletionCall, completion};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use secrecy::SecretString;
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;

use super::harness::{
    CountingCounter, RecordingFactory, RecordingResolver, RecordingSleeper, ScriptedTransport,
    secret_headers, shown_headers,
};
use super::input::{DispatchCall, Step};
use super::views::reply_view;

type Calls = Result<Value, Box<HelperError>>;
type Answer = Result<(), Box<HelperError>>;

pub(super) fn dispatches(calls: &[DispatchCall], runtime: &Runtime) -> Calls {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(runtime.block_on(dispatch_answer(call))?);
    }
    Ok(Value::Array(answers))
}

async fn dispatch_answer(call: &DispatchCall) -> Calls {
    let headers = secret_headers(&call.headers);
    let environment = BTreeMap::new();
    let resolver = RecordingResolver::new(call.token.as_deref());
    let key = call.api_key.as_deref().map(SecretString::from);
    let route = dispatch(
        FoundryDispatchCall {
            model: &call.model,
            messages: &call.messages,
            optional_params: &call.optional_params,
            litellm_params: &call.litellm_params,
            headers: &headers,
            environment: &environment,
            api_base: Some(&call.api_base),
            api_key: key.as_ref(),
            reply: AgentReply::default(),
            acompletion: call.acompletion,
            stream: call.stream,
        },
        &resolver,
    );
    let mut answer = Map::new();
    answer.insert(String::from("resolver"), json!(resolver.seen()));
    match route {
        Err(error) => insert_failure(&mut answer, &error),
        Ok(Route::Stream(request)) => insert_stream(&mut answer, &request)?,
        Ok(Route::Completion(completion_call)) => {
            insert_completion(&mut answer, &completion_call, &call.steps).await?;
        }
    }
    Ok(Value::Object(answer))
}

fn insert_failure(answer: &mut Map<String, Value>, error: &AgentRunError) {
    answer.insert(String::from("ok"), json!(false));
    answer.insert(String::from("status"), json!(error.status()));
    answer.insert(String::from("message"), json!(error.to_string()));
}

fn insert_stream(answer: &mut Map<String, Value>, request: &StreamRequest) -> Answer {
    answer.insert(String::from("ok"), json!(true));
    answer.insert(String::from("route"), json!("stream"));
    answer.insert(String::from("url"), json!(request.url));
    answer.insert(
        String::from("headers"),
        json!(shown_headers(&request.headers)),
    );
    answer.insert(
        String::from("body"),
        serde_json::to_value(&request.payload).or_config_error()?,
    );
    answer.insert(
        String::from("ssl_verify"),
        serde_json::to_value(&request.ssl_verify).or_config_error()?,
    );
    Ok(())
}

async fn insert_completion(
    answer: &mut Map<String, Value>,
    completion_call: &CompletionCall<'_>,
    steps: &[Step],
) -> Answer {
    let scripted = ScriptedTransport::new(steps);
    let factory = RecordingFactory::new(&scripted);
    let sleeper = RecordingSleeper::default();
    match completion(completion_call, &factory, &sleeper, &CountingCounter).await {
        Ok(reply) => {
            answer.insert(String::from("ok"), json!(true));
            answer.insert(String::from("route"), json!("completion"));
            answer.insert(
                String::from("client"),
                json!(*factory.kind.lock().unwrap_or_else(PoisonError::into_inner)),
            );
            answer.insert(
                String::from("ssl_verify"),
                serde_json::to_value(&completion_call.ssl_verify).or_config_error()?,
            );
            answer.insert(String::from("requests"), scripted.requests());
            answer.insert(String::from("sleeps"), json!(sleeper.values()));
            answer.insert(
                String::from("outcome"),
                json!({"ok": true, "reply": reply_view(&reply)}),
            );
        }
        Err(error) => insert_failure(answer, &error),
    }
    Ok(())
}

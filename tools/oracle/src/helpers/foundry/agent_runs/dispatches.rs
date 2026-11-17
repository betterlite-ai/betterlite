use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_foundry::agent_results::reply::AgentReply;
use betterlite_foundry::agent_runs::dispatch::{
    DispatchCall as FoundryDispatchCall, Route, dispatch,
};
use betterlite_foundry::agent_runs::flow::completion;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::fixtures::{
    CountingCounter, RecordingFactory, RecordingResolver, RecordingSleeper, ScriptedTransport,
    secret_headers, shown_headers,
};
use super::input::DispatchCall;
use super::views::reply_view;

type Calls = Result<Value, Box<HelperError>>;

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
    let mut answer = serde_json::Map::new();
    answer.insert(String::from("resolver"), json!(resolver.seen()));
    match route {
        Err(error) => {
            answer.insert(String::from("ok"), json!(false));
            answer.insert(String::from("status"), json!(error.status()));
            answer.insert(String::from("message"), json!(error.to_string()));
        }
        Ok(Route::Stream(request)) => {
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
        }
        Ok(Route::Completion(completion_call)) => {
            let scripted = Arc::new(ScriptedTransport::new(&call.steps));
            let factory = RecordingFactory::new(&scripted);
            let sleeper = RecordingSleeper::default();
            let done = completion(&completion_call, &factory, &sleeper, &CountingCounter).await;
            match done {
                Ok(reply) => {
                    answer.insert(String::from("ok"), json!(true));
                    answer.insert(String::from("route"), json!("completion"));
                    let client = *factory.kind.lock().unwrap_or_else(PoisonError::into_inner);
                    answer.insert(String::from("client"), json!(client));
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
                Err(error) => {
                    answer.insert(String::from("ok"), json!(false));
                    answer.insert(String::from("status"), json!(error.status()));
                    answer.insert(String::from("message"), json!(error.to_string()));
                }
            }
        }
    }
    Ok(Value::Object(answer))
}

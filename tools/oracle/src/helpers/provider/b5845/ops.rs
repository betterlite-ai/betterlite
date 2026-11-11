use std::sync::{Mutex, PoisonError};

use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::sap_chat::environment::SapChatConfig;
use betterlite_provider::sap_chat::params::get_supported_openai_params as chat_params;
use betterlite_provider::sap_chat::request::transform_request;
use betterlite_provider::sap_chat::response::{
    PostCall, PostCallLogger, ResponseInput, transform_response,
};
use betterlite_provider::sap_embedding::environment::SapEmbeddingConfig;
use betterlite_provider::sap_embedding::error::get_error_class;
use betterlite_provider::sap_embedding::params::{
    get_supported_openai_params as embedding_params, map_openai_params,
};
use betterlite_provider::sap_embedding::request::transform_embedding_request;
use betterlite_provider::sap_embedding::response::transform_embedding_response;
use http::HeaderMap;
use serde_json::{Value, json};

use super::input::{
    ChatRequestInput, ChatResponseInput, EmbeddingCall, EnvironmentCall, FixtureEnvironment, Input,
    ModelInput, session_parts,
};

fn failure(status: Option<u16>) -> Value {
    json!({
        "ok": false,
        "status": status,
    })
}

fn succeeded(value: &Value) -> Value {
    json!({
        "ok": true,
        "value": value,
    })
}

fn collect_outcomes<T, F>(calls: &[T], run: F) -> Vec<Value>
where
    F: Fn(&T) -> Value,
{
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        outcomes.push(run(call));
    }
    outcomes
}

fn prepared_outcome(prepared: Result<(HeaderMap, String), Option<u16>>) -> Value {
    prepared.map_or_else(failure, |(headers, url)| {
        succeeded(&json!({
            "headers": header_pairs(&headers),
            "url": url,
        }))
    })
}

fn supported_output(calls: &[ModelInput]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                let params = if call.family.as_deref() == Some("embedding") {
                    embedding_params(&call.model)
                } else {
                    chat_params(&call.model)
                };
                succeeded(&json!(params))
            })
            .collect(),
    )
}

fn requests_output(calls: &[ChatRequestInput]) -> Value {
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        let result = transform_request(&call.model, &call.messages, &call.optional_params);
        outcomes.push(result.map_or_else(
            |error| failure(error.status()),
            |body| succeeded(&json!(body)),
        ));
    }
    Value::Array(outcomes)
}

struct Recorder {
    calls: Mutex<Vec<Value>>,
}

impl PostCallLogger for Recorder {
    fn post_call(&self, call: PostCall<'_>) {
        let record = json!({
            "input": call.input,
            "api_key": call.api_key,
            "original_response": call.original_response,
            "complete_input_dict": call.request_data,
        });
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

fn response_view(response: &ChatResponse) -> Value {
    let choices: Vec<Value> = response
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "message": {
                    "role": choice.message.role,
                    "content": choice.message.content,
                },
            })
        })
        .collect();
    json!({
        "id": response.id,
        "object": response.object,
        "created": response.created,
        "model": response.model,
        "choices": choices,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    })
}

fn responses_output(calls: &[ChatResponseInput]) -> Value {
    Value::Array(collect_outcomes(calls, |call| {
        let empty = Json::Object(Object::new());
        let recorder = Recorder {
            calls: Mutex::new(Vec::new()),
        };
        let input = ResponseInput {
            model: &call.model,
            messages: &call.messages,
            api_key: call.api_key.as_deref(),
            raw_response: &call.raw_text,
            request_data: call.request_data.as_ref().unwrap_or(&empty),
            response_format: call.response_format.as_ref(),
        };
        match transform_response(&input, &recorder) {
            Ok(response) => {
                let logged = recorder
                    .calls
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                succeeded(&json!({
                    "response": response_view(&response),
                    "post_call": logged,
                }))
            }
            Err(error) => failure(error.status()),
        }
    }))
}

fn header_pairs(headers: &HeaderMap) -> Value {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            let text = value.to_str().unwrap_or_default().to_owned();
            (name.as_str().to_owned(), text)
        })
        .collect();
    pairs.sort();
    Value::Array(
        pairs
            .into_iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}

fn environment_outcome(call: &EnvironmentCall) -> Value {
    let environment = FixtureEnvironment {
        input: &call.environment,
    };
    let credentials = match SapChatConfig::setup(call.service_key.as_ref(), &environment) {
        Ok(found) => found,
        Err(error) => return failure(error.status()),
    };
    let (token, resource_group, deployment_url) = session_parts(call.session.as_ref());
    // the key's base_url drives the deployment lookup in B8331; this helper uses the case's URL
    let config = SapChatConfig {
        token,
        resource_group: credentials.map_or(resource_group, |found| found.resource_group),
        deployment_url,
    };
    prepared_outcome(
        config
            .prepare()
            .map(|prepared| (prepared.headers, prepared.url))
            .map_err(|error| error.status()),
    )
}

fn environments_output(calls: &[EnvironmentCall]) -> Value {
    let mut outcomes = Vec::with_capacity(calls.len());
    outcomes.extend(calls.iter().map(environment_outcome));
    Value::Array(outcomes)
}

fn embedding_call_output(call: &EmbeddingCall) -> Value {
    match call {
        EmbeddingCall::Request {
            model,
            input,
            optional_params,
        } => transform_embedding_request(model, input, optional_params)
            .map_or_else(|_| failure(None), |body| succeeded(&json!(body))),
        EmbeddingCall::MapParams {
            model,
            non_default_params,
            optional_params,
            drop_params,
        } => succeeded(&json!(map_openai_params(
            non_default_params,
            optional_params.clone(),
            model,
            *drop_params,
        ))),
        EmbeddingCall::Prepare { session } => {
            let (token, resource_group, deployment_url) = session_parts(session.as_ref());
            let config = SapEmbeddingConfig {
                token,
                resource_group,
                deployment_url,
            };
            prepared_outcome(
                config
                    .prepare()
                    .map(|prepared| (prepared.headers, prepared.url))
                    .map_err(|_| None),
            )
        }
        EmbeddingCall::Response { raw_text } => transform_embedding_response(raw_text)
            .map_or_else(|_| failure(None), |response| succeeded(&json!(response))),
        EmbeddingCall::ErrorClass { message, status } => {
            let error = get_error_class(message, *status, &HeaderMap::new());
            succeeded(&json!({"status": error.status_code, "message": error.message}))
        }
    }
}

pub(crate) fn output(input: &Input) -> Value {
    match input {
        Input::SupportedParams(calls) => supported_output(calls),
        Input::Requests(calls) => requests_output(calls),
        Input::Responses(calls) => responses_output(calls),
        Input::Environments(calls) => environments_output(calls),
        Input::Embeddings(calls) => Value::Array(collect_outcomes(calls, embedding_call_output)),
    }
}

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::sap_auth::environment::SapEnvironment;
use betterlite_provider::sap_chat::environment::SapChatConfig;
use betterlite_provider::sap_chat::params::get_supported_openai_params as chat_params;
use betterlite_provider::sap_chat::request::transform_request;
use betterlite_provider::sap_chat::response::{
    PostCall, PostCallLogger, ResponseInput, transform_response,
};
use betterlite_provider::sap_embedding::environment::SapEmbeddingConfig;
use betterlite_provider::sap_embedding::error::get_error_class;
use betterlite_provider::sap_embedding::params::get_supported_openai_params as embedding_params;
use betterlite_provider::sap_embedding::request::transform_embedding_request;
use betterlite_provider::sap_embedding::response::transform_embedding_response;
use http::HeaderMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperResult;

const DEFAULT_RESOURCE_GROUP: &str = "default";
const DEFAULT_HOME: &str = "/nonexistent-home";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Family {
    Chat,
    Embedding,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelInput {
    model: String,
    #[serde(default = "default_family")]
    family: Family,
}

const fn default_family() -> Family {
    Family::Chat
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatRequestInput {
    model: String,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatResponseInput {
    model: String,
    raw_text: String,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    request_data: Option<Json>,
    #[serde(default)]
    response_format: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionInput {
    token: String,
    #[serde(default)]
    resource_group: Option<String>,
    deployment_url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentInput {
    #[serde(default)]
    vars: BTreeMap<String, String>,
    #[serde(default)]
    files: BTreeMap<String, String>,
    #[serde(default)]
    home: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentCall {
    #[serde(default)]
    service_key: Option<Json>,
    #[serde(default)]
    environment: EnvironmentInput,
    #[serde(default)]
    session: Option<SessionInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EmbeddingCall {
    Request {
        model: String,
        input: EmbeddingInput,
        #[serde(default)]
        optional_params: Object,
    },
    Prepare {
        #[serde(default)]
        session: Option<SessionInput>,
    },
    Response {
        raw_text: String,
    },
    ErrorClass {
        message: String,
        status: u16,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "calls", rename_all = "snake_case")]
pub(crate) enum Input {
    SupportedParams(Vec<ModelInput>),
    Requests(Vec<ChatRequestInput>),
    Responses(Vec<ChatResponseInput>),
    Environments(Vec<EnvironmentCall>),
    Embeddings(Vec<EmbeddingCall>),
}

struct FixtureEnvironment {
    files: BTreeMap<String, String>,
    vars: BTreeMap<String, String>,
    home: PathBuf,
}

impl SapEnvironment for FixtureEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn file_exists(&self, path: &Path) -> bool {
        path.to_str()
            .is_some_and(|text| self.files.contains_key(text))
    }

    fn read_file(&self, path: &Path) -> Result<String, io::Error> {
        path.to_str()
            .and_then(|text| self.files.get(text))
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn home_dir(&self) -> PathBuf {
        self.home.clone()
    }
}

impl EnvironmentInput {
    fn fixture(&self) -> FixtureEnvironment {
        FixtureEnvironment {
            files: self.files.clone(),
            vars: self.vars.clone(),
            home: PathBuf::from(
                self.home
                    .clone()
                    .unwrap_or_else(|| String::from(DEFAULT_HOME)),
            ),
        }
    }
}

impl SessionInput {
    fn chat_config(&self) -> SapChatConfig {
        SapChatConfig {
            token: SecretString::from(self.token.clone()),
            resource_group: self
                .resource_group
                .clone()
                .unwrap_or_else(|| String::from(DEFAULT_RESOURCE_GROUP)),
            deployment_url: self.deployment_url.clone(),
        }
    }

    fn embedding_config(&self) -> SapEmbeddingConfig {
        SapEmbeddingConfig {
            token: SecretString::from(self.token.clone()),
            resource_group: self
                .resource_group
                .clone()
                .unwrap_or_else(|| String::from(DEFAULT_RESOURCE_GROUP)),
            deployment_url: self.deployment_url.clone(),
        }
    }
}

fn placeholder_chat_config() -> SapChatConfig {
    SapChatConfig {
        token: SecretString::from(String::from("Bearer no-token")),
        resource_group: String::from(DEFAULT_RESOURCE_GROUP),
        deployment_url: String::new(),
    }
}

fn placeholder_embedding_config() -> SapEmbeddingConfig {
    SapEmbeddingConfig {
        token: SecretString::from(String::from("Bearer no-token")),
        resource_group: String::from(DEFAULT_RESOURCE_GROUP),
        deployment_url: String::new(),
    }
}

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

fn supported_output(calls: &[ModelInput]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                let params = match call.family {
                    Family::Chat => chat_params(&call.model),
                    Family::Embedding => embedding_params(&call.model),
                };
                succeeded(&json!(params))
            })
            .collect(),
    )
}

fn requests_output(calls: &[ChatRequestInput]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                match transform_request(&call.model, &call.messages, &call.optional_params) {
                    Ok(body) => succeeded(&json!(body)),
                    Err(error) => failure(error.status()),
                }
            })
            .collect(),
    )
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
    json!({
        "id": response.id,
        "object": response.object,
        "created": response.created,
        "model": response.model,
        "choices": response
            .choices
            .iter()
            .map(|choice| json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "message": {
                    "role": choice.message.role,
                    "content": choice.message.content,
                },
            }))
            .collect::<Vec<_>>(),
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    })
}

fn responses_output(calls: &[ChatResponseInput]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                let empty = Json::Object(Object::new());
                let request_data = call.request_data.as_ref().unwrap_or(&empty);
                let recorder = Recorder {
                    calls: Mutex::new(Vec::new()),
                };
                let input = ResponseInput {
                    model: &call.model,
                    messages: &call.messages,
                    api_key: call.api_key.as_deref(),
                    raw_response: &call.raw_text,
                    request_data,
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
            })
            .collect(),
    )
}

fn header_pairs(headers: &HeaderMap) -> Value {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
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

fn environments_output(calls: &[EnvironmentCall]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                let config = call
                    .session
                    .as_ref()
                    .map_or_else(placeholder_chat_config, SessionInput::chat_config);
                let environment = call.environment.fixture();
                match config.prepare(call.service_key.as_ref(), &environment) {
                    Ok(prepared) => succeeded(&json!({
                        "headers": header_pairs(&prepared.headers),
                        "url": prepared.url,
                    })),
                    Err(error) => failure(error.status()),
                }
            })
            .collect(),
    )
}

fn embedding_view(response: &EmbeddingResponse) -> Value {
    json!({
        "object": response.object,
        "model": response.model,
        "data": response
            .data
            .iter()
            .map(|item| json!({
                "object": item.object,
                "embedding": item.embedding,
                "index": item.index,
            }))
            .collect::<Vec<_>>(),
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    })
}

fn embedding_call_output(call: &EmbeddingCall) -> Value {
    match call {
        EmbeddingCall::Request {
            model,
            input,
            optional_params,
        } => transform_embedding_request(model, input, optional_params)
            .map_or_else(|_| failure(None), |body| succeeded(&json!(body))),
        EmbeddingCall::Prepare { session } => {
            let config = session
                .as_ref()
                .map_or_else(placeholder_embedding_config, SessionInput::embedding_config);
            match config.prepare() {
                Ok(prepared) => succeeded(&json!({
                    "headers": header_pairs(&prepared.headers),
                    "url": prepared.url,
                })),
                Err(_) => failure(None),
            }
        }
        EmbeddingCall::Response { raw_text } => transform_embedding_response(raw_text).map_or_else(
            |_| failure(None),
            |response| succeeded(&embedding_view(&response)),
        ),
        EmbeddingCall::ErrorClass { message, status } => {
            let error = get_error_class(message, *status, &HeaderMap::new());
            succeeded(&json!({"status": error.status_code, "message": error.message}))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let value = match input {
            Input::SupportedParams(calls) => supported_output(calls),
            Input::Requests(calls) => requests_output(calls),
            Input::Responses(calls) => responses_output(calls),
            Input::Environments(calls) => environments_output(calls),
            Input::Embeddings(calls) => {
                Value::Array(calls.iter().map(embedding_call_output).collect())
            }
        };
        Ok(value)
    }
}

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use betterlite_openai::text_completion_transport::sdk::{RawCompletion, SdkError, StatusError};
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use chrono::{DateTime, Utc};
use futures_util::stream;
use http::HeaderMap;
use secrecy::SecretString;
use uuid::Uuid;

use crate::completion::client::{
    ClientHandle, ClientSource, CompletionsTransport, GatewayClient, GetClient,
};
use crate::completion::logging::CompletionHooks;
use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::ClientKind;

pub const CLOCK: i64 = 1_700_000_000;

pub fn object(source: &str) -> Extra {
    match serde_json::from_str::<Json>(source).unwrap() {
        Json::Object(fields) => fields,
        _ => panic!("not an object: {source}"),
    }
}

#[derive(Clone)]
pub enum Answer {
    Buffered(String),
    Stream(Vec<String>),
    Status {
        status: u16,
        message: String,
        headers: Vec<(String, String)>,
    },
    Other(String),
}

#[derive(Clone)]
pub struct ScriptedTransport {
    pub answer: Arc<Mutex<Answer>>,
    pub creates: Arc<Mutex<Vec<Json>>>,
}

fn status_error(status: u16, message: &str, headers: &[(String, String)]) -> SdkError {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (
            name.parse::<http::header::HeaderName>(),
            value.parse::<http::header::HeaderValue>(),
        ) {
            map.insert(name, value);
        }
    }
    SdkError::Status(Box::new(StatusError {
        status: Some(status),
        message: message.to_owned(),
        headers: map,
        error_type: None,
        param: None,
        code: None,
    }))
}

impl CompletionsTransport for ScriptedTransport {
    fn create<'a>(
        &'a self,
        data: &'a Extra,
        timeout: Option<f64>,
    ) -> Pin<Box<dyn Future<Output = Result<RawCompletion, SdkError>> + Send + 'a>> {
        let mut record = Extra::new();
        record.insert("data".to_owned(), Json::Object(data.clone()));
        record.insert(
            "timeout".to_owned(),
            timeout.map_or(Json::Null, |value| {
                Json::Number(serde_json::Number::from_f64(value).unwrap())
            }),
        );
        self.creates.lock().unwrap().push(Json::Object(record));
        let answer = self.answer.lock().unwrap().clone();
        Box::pin(async move {
            match answer {
                Answer::Buffered(body) => Ok(RawCompletion::Buffered {
                    status: 200,
                    headers: HeaderMap::new(),
                    body: body.into_bytes(),
                }),
                Answer::Stream(chunks) => {
                    let items: Vec<Result<Vec<u8>, SdkError>> = chunks
                        .into_iter()
                        .map(|chunk| Ok(chunk.into_bytes()))
                        .collect();
                    Ok(RawCompletion::Stream {
                        status: 200,
                        headers: HeaderMap::new(),
                        body: Box::pin(stream::iter(items)),
                    })
                }
                Answer::Status {
                    status,
                    message,
                    headers,
                } => Err(status_error(status, &message, &headers)),
                Answer::Other(message) => Err(SdkError::Other { message }),
            }
        })
    }
}

#[derive(Clone)]
pub struct ScriptedSource {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub base_url: String,
    pub fail_get: bool,
    pub answer: Arc<Mutex<Answer>>,
    pub gets: Arc<Mutex<Vec<Json>>>,
    pub gateways: Arc<Mutex<Vec<Json>>>,
    pub creates: Arc<Mutex<Vec<Json>>>,
}

impl ScriptedSource {
    pub fn new(kind: ClientKind, answer: Answer) -> Self {
        Self {
            kind,
            api_key: Some(SecretString::from("sk-unit")),
            base_url: "https://unit.test/openai/".to_owned(),
            fail_get: false,
            answer: Arc::new(Mutex::new(answer)),
            gets: Arc::new(Mutex::new(Vec::new())),
            gateways: Arc::new(Mutex::new(Vec::new())),
            creates: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn handle(&self) -> ClientHandle<ScriptedTransport> {
        ClientHandle {
            kind: self.kind,
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            transport: ScriptedTransport {
                answer: self.answer.clone(),
                creates: self.creates.clone(),
            },
        }
    }

    pub fn creates(&self) -> Vec<Json> {
        self.creates.lock().unwrap().clone()
    }

    pub fn gets(&self) -> Vec<Json> {
        self.gets.lock().unwrap().clone()
    }

    pub fn gateways(&self) -> Vec<Json> {
        self.gateways.lock().unwrap().clone()
    }
}

fn text_or_null(value: Option<&str>) -> Json {
    match value {
        Some(text) => Json::Text(text.to_owned()),
        None => Json::Null,
    }
}

impl ClientSource for ScriptedSource {
    type Client = ScriptedTransport;

    fn get(&self, call: GetClient<'_>) -> Result<ClientHandle<Self::Client>, EndpointError> {
        let mut record = Extra::new();
        record.insert("model".to_owned(), text_or_null(call.model));
        record.insert("api_base".to_owned(), text_or_null(call.api_base));
        record.insert("is_async".to_owned(), Json::Bool(call.is_async));
        record.insert(
            "max_retries".to_owned(),
            call.litellm_params
                .max_retries
                .map_or(Json::Null, |value| Json::Number(value.into())),
        );
        self.gets.lock().unwrap().push(Json::Object(record));
        if self.fail_get {
            return Err(EndpointError::MissingApiVersion);
        }
        Ok(self.handle())
    }

    fn gateway(
        &self,
        call: GatewayClient<'_>,
    ) -> Result<ClientHandle<Self::Client>, EndpointError> {
        let mut record = Extra::new();
        record.insert("api_base".to_owned(), Json::Text(call.api_base.to_owned()));
        record.insert("model".to_owned(), Json::Text(call.model.to_owned()));
        record.insert("max_retries".to_owned(), call.max_retries.value().clone());
        record.insert("acompletion".to_owned(), Json::Bool(call.acompletion));
        self.gateways.lock().unwrap().push(Json::Object(record));
        Ok(self.handle())
    }
}

#[derive(Default)]
pub struct Recorder {
    pub pre: Vec<Json>,
    pub post: Vec<Json>,
    pub fail_pre: bool,
    pub hidden: JsonObject,
}

fn version_args(args: &AdditionalArgs) -> Extra {
    let mut fields = Extra::new();
    fields.insert(
        "api_base".to_owned(),
        args.api_base
            .as_ref()
            .map_or(Json::Null, |base| Json::Text(base.clone())),
    );
    fields.insert(
        "headers".to_owned(),
        args.headers
            .as_ref()
            .map_or(Json::Null, |headers| Json::Object(headers.clone())),
    );
    if let Some(body) = &args.complete_input_dict {
        fields.insert("complete_input_dict".to_owned(), Json::from(body));
    }
    for (name, value) in &args.passthrough {
        fields.insert(name.clone(), value.clone());
    }
    Json::Object(fields)
}

impl CompletionHooks for Recorder {
    fn pre_call(&mut self, call: PreCall) -> Result<(), PrePostError> {
        if self.fail_pre {
            return Err(PrePostError::CallbackFailed {
                message: "hook failed".to_owned(),
            });
        }
        let mut fields = Extra::new();
        fields.insert("input".to_owned(), call.input);
        fields.insert(
            "model".to_owned(),
            call.model.map_or(Json::Null, Json::Text),
        );
        fields.insert(
            "additional_args".to_owned(),
            version_args(&call.additional_args),
        );
        self.pre.push(Json::Object(fields));
        Ok(())
    }

    fn post_call(&mut self, call: PostCall) {
        let mut fields = Extra::new();
        fields.insert("input".to_owned(), call.input);
        fields.insert("original_response".to_owned(), call.original_response);
        fields.insert(
            "additional_args".to_owned(),
            version_args(&call.additional_args),
        );
        self.post.push(Json::Object(fields));
    }

    fn stream_defaults(&self) -> StreamDefaults {
        StreamDefaults {
            stream_options: None,
            merge_reasoning_in_choices: false,
            is_function_call: false,
            base_hidden_params: self.hidden.clone(),
        }
    }
}

pub struct Fixed {
    pub factory: PromptFactory,
    pub now: DateTime<Utc>,
}

impl Fixed {
    pub fn new() -> Self {
        Self {
            factory: PromptFactory {
                cache: Mutex::new(builtin_entries()),
                hub: Hub::Unreachable,
            },
            now: DateTime::from_timestamp(CLOCK, 0).unwrap(),
        }
    }
}

impl Default for Fixed {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run<Fut: Future>(future: Fut) -> Fut::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

pub fn identifier() -> Uuid {
    Uuid::from_u128(1)
}

pub fn clock() -> i64 {
    CLOCK
}

pub fn stream_now() -> StreamInstant {
    StreamInstant {
        seconds: CLOCK,
        timestamp: CLOCK as f64,
    }
}

pub fn stream_id() -> Uuid {
    Uuid::from_u128(2)
}

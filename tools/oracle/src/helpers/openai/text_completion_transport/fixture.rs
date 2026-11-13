use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::text_completion_policy::transform_completion_request;
use betterlite_openai::text_completion_transport::TextCompletionConfigs;
use betterlite_openai::text_completion_transport::logging::{
    CompletionLogger, PostCallRecord, PreCallRecord, ResponseKind, StreamDefaults,
};
use betterlite_openai::text_completion_transport::sdk::{
    ByteStream, ClientConstructor, ClientKind, RawCompletion, SdkArguments, SdkError, StatusError,
    TextCompletionSdk,
};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub(super) type Log = Arc<Mutex<Vec<Value>>>;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Failure {
    status: Option<u16>,
    message: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Answer {
    Buffered(Value),
    Stream {
        chunks: Vec<Value>,
        failure: Option<Failure>,
    },
    Failure(Failure),
}

impl Failure {
    fn error(&self) -> SdkError {
        let Some(status) = self.status else {
            return SdkError::Other {
                message: self.message.clone(),
            };
        };
        let mut headers = HeaderMap::new();
        for (field, text) in &self.headers {
            if let (Ok(name), Ok(value)) =
                (field.parse::<HeaderName>(), text.parse::<HeaderValue>())
            {
                headers.insert(name, value);
            }
        }
        SdkError::Status(Box::new(StatusError {
            status: Some(status),
            message: self.message.clone(),
            headers,
            error_type: None,
            param: None,
            code: None,
        }))
    }
}

#[derive(Clone)]
pub(super) struct Sdk {
    pub(super) kind: ClientKind,
    pub(super) calls: Log,
    pub(super) answer: Answer,
}

impl TextCompletionSdk for Sdk {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    fn create(
        &self,
        data: &Map<String, Value>,
    ) -> impl Future<Output = Result<RawCompletion, SdkError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Value::Object(data.clone()));
        ready(match &self.answer {
            Answer::Failure(failure) => Err(failure.error()),
            Answer::Buffered(body) => Ok(RawCompletion::Buffered {
                status: 200,
                headers: HeaderMap::new(),
                body: body.to_string().into_bytes(),
            }),
            Answer::Stream { chunks, failure } => {
                let mut items: Vec<Result<Vec<u8>, SdkError>> = chunks
                    .iter()
                    .map(|chunk| Ok(format!("data: {chunk}\n\n").into_bytes()))
                    .collect();
                items.push(failure.as_ref().map_or_else(
                    || Ok(b"data: [DONE]\n\n".to_vec()),
                    |raised| Err(raised.error()),
                ));
                let body: ByteStream = Box::pin(stream::iter(items));
                Ok(RawCompletion::Stream {
                    status: 200,
                    headers: HeaderMap::new(),
                    body,
                })
            }
        })
    }
}

pub(super) struct Constructor {
    pub(super) built: Log,
    pub(super) calls: Log,
    pub(super) answer: Answer,
}

impl Constructor {
    pub(super) fn logs(&self) -> (Vec<Value>, Vec<Value>) {
        let built = self.built.lock().unwrap_or_else(PoisonError::into_inner);
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        (built.clone(), calls.clone())
    }
}

impl ClientConstructor for Constructor {
    type Client = Sdk;

    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<Sdk>, SdkError> {
        self.built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "kind": kind,
                "http_client": arguments.http_client,
                "api_key": arguments.api_key.as_ref().map(ExposeSecret::expose_secret),
                "base_url": arguments.base_url,
                "timeout": arguments.timeout,
                "max_retries": arguments.max_retries,
                "organization": arguments.organization,
            }));
        Ok(Some(Sdk {
            kind,
            calls: Arc::clone(&self.calls),
            answer: self.answer.clone(),
        }))
    }
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) pre: Vec<Value>,
    pub(super) post: Vec<Value>,
}

impl CompletionLogger for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(json!({
            "input": record.input,
            "api_key": record.api_key.as_ref().map(ExposeSecret::expose_secret),
            "headers": record.headers.0,
            "api_base": record.api_base,
            "complete_input_dict": record.complete_input_dict,
        }));
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(json!({
            "api_key": record.api_key.as_ref().map(ExposeSecret::expose_secret),
            "original_response": record.original_response,
            "is_sdk_object": record.kind == ResponseKind::SdkObject,
            "headers": record.headers.0,
            "api_base": record.api_base,
        }));
    }

    fn stream_defaults(&self) -> StreamDefaults {
        StreamDefaults {
            stream_options: None,
            merge_reasoning_in_choices: false,
            is_function_call: false,
            base_hidden_params: JsonObject::new(),
        }
    }
}

pub(super) const fn configs() -> TextCompletionConfigs {
    TextCompletionConfigs {
        open_ai: transform_completion_request,
        fireworks_ai: transform_completion_request,
        together_ai: transform_completion_request,
        text_completion_inception: transform_completion_request,
    }
}

pub(super) const fn instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000_i64,
        timestamp: 1_700_000_000.25_f64,
    }
}

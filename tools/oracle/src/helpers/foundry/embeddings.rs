use std::future::{Future, ready};
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_azure::image_transport::{ImageReply, ImageRequest, ImageTransport};
use betterlite_foundry::embeddings::call::{
    EmbeddingClientFactory, FoundryEmbeddingRequest, SuppliedClient, TextEmbedding,
    TextEmbeddingCall, embedding, projected_headers,
};
use betterlite_foundry::embeddings::error::FoundryEmbeddingError;
use betterlite_foundry::embeddings::reply::{
    EmbeddingReply, FoundryEmbeddingResponse, decode_foundry_embedding,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Value, json};

mod input;
mod tests;

use input::{ClientName, FlowCall, Input, ReplyCall};

#[derive(Default)]
struct Trail {
    calls: Vec<Value>,
    constructed: Vec<Value>,
}

#[derive(Default)]
struct State {
    trail: Mutex<Trail>,
}

type Shared = Arc<State>;

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn from_call(call: &ReplyCall) -> Self {
        Self {
            status: call.status,
            headers: call
                .headers
                .iter()
                .map(|header| (header.name.clone(), header.value.clone()))
                .collect(),
            body: call.body.clone(),
        }
    }

    fn image_reply(&self) -> ImageReply {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(name, value);
            }
        }
        ImageReply {
            status: self.status,
            headers,
            body: self.body.as_bytes().to_vec(),
        }
    }
}

// the route's image transport and B173's text call share one fixture, and it records each request
#[derive(Clone)]
struct Recorder {
    state: Shared,
    reply: Reply,
}

impl ImageTransport for Recorder {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        let authorization = request
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        self.state
            .trail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .push(json!({
                "kind": "image",
                "url": request.url,
                "authorization": authorization,
                "body": body,
            }));
        ready(Ok(self.reply.image_reply()))
    }
}

struct Factory {
    state: Shared,
    reply: Reply,
}

impl Factory {
    fn build(&self, kind: &str, timeout: Duration) -> Recorder {
        self.state
            .trail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .constructed
            .push(json!({"kind": kind, "timeout": timeout.as_secs_f64()}));
        Recorder {
            state: Arc::clone(&self.state),
            reply: self.reply.clone(),
        }
    }
}

impl EmbeddingClientFactory for Factory {
    type Client = Recorder;

    fn sync_client(&self, timeout: Duration) -> Self::Client {
        self.build("sync", timeout)
    }

    fn async_client(&self, timeout: Duration) -> Self::Client {
        self.build("async", timeout)
    }
}

impl TextEmbedding for Recorder {
    fn embed(
        &self,
        call: TextEmbeddingCall<'_>,
    ) -> impl Future<Output = Result<EmbeddingReply, FoundryEmbeddingError>> + Send {
        let options = serde_json::to_value(call.options);
        self.state
            .trail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .push(json!({
                "kind": "text",
                "model": call.model,
                "input": call.input,
                "options": options.unwrap_or(Value::Null),
            }));
        let decoded = decode_foundry_embedding(self.reply.body.as_bytes());
        let mut reply = match decoded {
            Ok(reply) => reply,
            Err(error) => return ready(Err(error)),
        };
        reply.additional_headers = projected_headers(&self.reply.image_reply().headers);
        ready(Ok(reply))
    }
}

// the Python class each failure corresponds to in LiteLLM's conversion path
enum Failure {
    Class(&'static str),
    Status(u16),
}

fn failure(error: &FoundryEmbeddingError) -> Option<Failure> {
    match error {
        FoundryEmbeddingError::ApiBaseMissing
        | FoundryEmbeddingError::ApiKeyMissing
        | FoundryEmbeddingError::TokenCount { .. } => Some(Failure::Class("ValueError")),
        FoundryEmbeddingError::ImageDataMissing | FoundryEmbeddingError::TextDataMissing => {
            Some(Failure::Class("Exception"))
        }
        FoundryEmbeddingError::Provider { status } => Some(Failure::Status(*status)),
        FoundryEmbeddingError::Json { source } if source.is_data() => {
            Some(Failure::Class("ResponseValidationError"))
        }
        FoundryEmbeddingError::Json { .. } => Some(Failure::Class("JSONDecodeError")),
        FoundryEmbeddingError::DataMissing => Some(Failure::Class("KeyError")),
        FoundryEmbeddingError::TooFewEntries { .. } => Some(Failure::Class("IndexError")),
        FoundryEmbeddingError::Transport { .. } => Some(Failure::Class("TransportError")),
        FoundryEmbeddingError::NotAnObject
        | FoundryEmbeddingError::FieldType { .. }
        | FoundryEmbeddingError::Encode { .. }
        | FoundryEmbeddingError::Header { .. }
        | FoundryEmbeddingError::Text(_) => None,
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn response_value(response: &FoundryEmbeddingResponse) -> Value {
    json!({
        "object": &response.object,
        "model": &response.model,
        "data": &response.data,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    })
}

fn flow_answer(calls: Vec<Value>, constructed: Vec<Value>, response: Value) -> Value {
    json!({
        "calls": calls,
        "constructed": constructed,
        "response": response,
        "error": null,
    })
}

fn flow_failure(calls: Vec<Value>, constructed: Vec<Value>, failure: &Failure) -> Value {
    let (kind, status) = match *failure {
        Failure::Class(kind) => (kind, Value::Null),
        Failure::Status(status) => ("HTTPStatusError", json!(status)),
    };
    json!({
        "calls": calls,
        "constructed": constructed,
        "response": null,
        "error": {"kind": kind, "status": status},
    })
}

fn flow_error(
    calls: Vec<Value>,
    constructed: Vec<Value>,
    error: &FoundryEmbeddingError,
) -> Result<Value, Box<HelperError>> {
    let Some(failure) = failure(error) else {
        return Err(helper_error("EmbeddingError", error.to_string()));
    };
    Ok(flow_failure(calls, constructed, &failure))
}

// the caller's client, when it hands the route one, answers the image call and records it
struct Fixtures {
    state: Shared,
    held: Option<Recorder>,
    factory: Factory,
}

fn fixtures(flow: &FlowCall) -> Fixtures {
    let state = Shared::default();
    let held = matches!(flow.client, ClientName::Handler).then(|| Recorder {
        state: Arc::clone(&state),
        reply: Reply::from_call(&flow.image_reply),
    });
    Fixtures {
        held,
        factory: Factory {
            state: Arc::clone(&state),
            reply: Reply::from_call(&flow.image_reply),
        },
        state,
    }
}

fn flow_outcome(
    flow: &FlowCall,
    runtime: &tokio::runtime::Runtime,
) -> Result<Value, Box<HelperError>> {
    let Fixtures {
        state,
        held,
        factory,
    } = fixtures(flow);
    let text = Recorder {
        state: Arc::clone(&state),
        reply: Reply::from_call(&flow.text_reply),
    };
    let key = flow.api_key.as_deref().map(SecretString::from);
    let supplied = held.as_ref().map(|client| SuppliedClient {
        mode: flow.mode.mode(),
        client,
    });
    let request = FoundryEmbeddingRequest {
        model: &flow.model,
        input: &flow.input,
        options: flow.options.options(),
        api_base: flow.api_base.as_deref(),
        api_key: key.as_ref(),
        timeout: Duration::from_secs(flow.timeout),
        mode: flow.mode.mode(),
    };
    let outcome = runtime.block_on(embedding(&request, supplied, &factory, &text));
    let (calls, constructed) = {
        let mut trail = state.trail.lock().unwrap_or_else(PoisonError::into_inner);
        (take(&mut trail.calls), take(&mut trail.constructed))
    };
    match outcome {
        Ok(response) => Ok(flow_answer(calls, constructed, response_value(&response))),
        Err(error) => flow_error(calls, constructed, &error),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", error.to_string()))?;
        let flows = input
            .flows
            .iter()
            .map(|flow| flow_outcome(flow, &runtime))
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(json!({ "flows": flows }))
    }
}

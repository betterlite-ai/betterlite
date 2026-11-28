use std::future::{Future, ready};
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_azure::image_transport::{ImageReply, ImageRequest, ImageTransport};
use betterlite_foundry::embeddings::call::{
    EmbeddingClientFactory, FoundryEmbeddingRequest, TextEmbedding, TextEmbeddingCall, embedding,
    projected_headers,
};
use betterlite_foundry::embeddings::error::FoundryEmbeddingError;
use betterlite_foundry::embeddings::reply::{
    EmbeddingReply, FoundryEmbeddingResponse, decode_foundry_embedding,
};
use betterlite_http::client_kind::SuppliedClient;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use http::header::{
    AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue,
};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

mod failure;
mod input;
mod tests;

use failure::{failure, helper_error};
use input::{ClientName, FlowCall, Input, ReplyCall};

#[derive(Default)]
struct Trail {
    calls: Vec<Value>,
    constructed: Vec<Value>,
}

type Shared = Arc<Mutex<Trail>>;

impl ReplyCall {
    fn image_reply(&self) -> ImageReply {
        let mut headers = HeaderMap::new();
        for header in &self.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(header.name.as_bytes()),
                HeaderValue::from_str(&header.value),
            ) {
                headers.append(name, value);
            }
        }
        // the Python fixture answers through httpx, which sets content-length on every reply
        if !headers.contains_key(CONTENT_LENGTH)
            && let Ok(value) = HeaderValue::from_str(&self.body.len().to_string())
        {
            headers.insert(CONTENT_LENGTH, value);
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
    reply: ReplyCall,
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
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .push(json!({
                "kind": "image",
                "url": request.url,
                "authorization": authorization,
                "content_type": request.headers.get(CONTENT_TYPE).and_then(|value| value.to_str().ok()),
                "body": body,
            }));
        ready(Ok(self.reply.image_reply()))
    }
}

struct Factory {
    state: Shared,
    reply: ReplyCall,
}

impl Factory {
    fn build(&self, kind: &str, timeout: Duration) -> Recorder {
        self.state
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

fn response_view(response: &FoundryEmbeddingResponse) -> Value {
    json!({
        "object": &response.object,
        "model": &response.model,
        "data": &response.data,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
        "additional_headers": &response.additional_headers,
    })
}

fn error_view(error: &FoundryEmbeddingError) -> Result<Value, Box<HelperError>> {
    let Some(mapped) = failure(error) else {
        return Err(helper_error("EmbeddingError", error.to_string()));
    };
    let status = mapped.status.map_or(Value::Null, |status| json!(status));
    Ok(json!({"kind": mapped.class, "status": status}))
}

// one flow's answer: the recorded calls and clients, then the response or the error class
fn flow_record(
    state: &Shared,
    outcome: Result<FoundryEmbeddingResponse, FoundryEmbeddingError>,
) -> Result<Value, Box<HelperError>> {
    let (calls, constructed) = {
        let mut trail = state.lock().unwrap_or_else(PoisonError::into_inner);
        (take(&mut trail.calls), take(&mut trail.constructed))
    };
    let (response, error) = match outcome {
        Ok(response) => (response_view(&response), Value::Null),
        Err(error) => (Value::Null, error_view(&error)?),
    };
    Ok(json!({
        "calls": calls,
        "constructed": constructed,
        "response": response,
        "error": error,
    }))
}

// the caller's client, when it hands the route one, answers the image call and records it
fn fixtures(flow: &FlowCall) -> (Shared, Option<Recorder>, Factory) {
    let state = Shared::default();
    let reply = flow.image_reply.clone();
    let held = matches!(flow.client, ClientName::Handler).then(|| Recorder {
        state: Arc::clone(&state),
        reply: reply.clone(),
    });
    let factory = Factory {
        state: Arc::clone(&state),
        reply,
    };
    (state, held, factory)
}

fn flow_outcome(flow: &FlowCall, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let (state, held, factory) = fixtures(flow);
    let text = Recorder {
        state: Arc::clone(&state),
        reply: flow.text_reply.clone(),
    };
    let key = flow.api_key.as_deref().map(SecretString::from);
    let supplied = held.as_ref().map(|client| SuppliedClient {
        kind: flow.mode,
        transport: client,
    });
    let request = FoundryEmbeddingRequest {
        model: &flow.model,
        input: &flow.input,
        options: flow.options.options(),
        api_base: flow.api_base.as_deref(),
        api_key: key.as_ref(),
        timeout: Duration::from_secs(flow.timeout),
        mode: flow.mode,
    };
    let outcome = runtime.block_on(embedding(&request, supplied, &factory, &text));
    flow_record(&state, outcome)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
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

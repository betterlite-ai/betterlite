#![cfg(test)]

use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Arc, Mutex};

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use thiserror::Error;
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::gemma_call::error::GemmaCallError;
use crate::gemma_call::execute::completion;
use crate::gemma_call::plan::{GemmaCall, Invocation, SuppliedClient, SuppliedClientKind};
use crate::gemma_envelope::post::{GemmaPost, PostFuture, PostReply, PostRequest, Timeout};
use crate::gemma_envelope::response::{GemmaReply, SingleResponseStreamFactory};
use crate::token_cache::probe::AccessToken;

pub(super) const MODEL: &str = "gemma/gemma-3-12b-it-1222199011122";
pub(super) const STRIPPED_MODEL: &str = "gemma-3-12b-it-1222199011122";
pub(super) const TOKEN: &str = "vertex-gemma-test-token";
pub(super) const ENDPOINT: &str = "https://ep.example/custom";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Event {
    Posted(String),
    Wrapped(String),
}

#[derive(Clone, Copy)]
pub(super) enum Reply {
    Ok(&'static str),
    Status(u16, &'static str),
}

#[derive(Clone, Debug)]
pub(super) struct SentRequest {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

#[derive(Debug, Error)]
#[error("the fixture transport never fails")]
pub(super) struct PostNever;

pub(super) type FixtureError = GemmaCallError<PostNever, PostNever>;

pub(super) struct FixtureClient {
    pub(super) reply: Reply,
    pub(super) sent: Arc<Mutex<Vec<SentRequest>>>,
    pub(super) events: Arc<Mutex<Vec<Event>>>,
}

impl GemmaPost for FixtureClient {
    type Failure = PostNever;

    fn post<'request>(
        &'request self,
        request: PostRequest<'request>,
        _timeout: Timeout,
    ) -> PostFuture<'request, Self::Failure> {
        self.sent.lock().unwrap().push(SentRequest {
            url: request.url.to_owned(),
            headers: request.headers.clone(),
            body: request.body.to_vec(),
        });
        self.events
            .lock()
            .unwrap()
            .push(Event::Posted(request.url.to_owned()));
        let (status, body) = match self.reply {
            Reply::Ok(body) => (200, body.as_bytes().to_vec()),
            Reply::Status(status, body) => (status, body.as_bytes().to_vec()),
        };
        Box::pin(ready(Ok(PostReply { status, body })))
    }
}

pub(super) struct Factory {
    pub(super) events: Arc<Mutex<Vec<Event>>>,
    pub(super) wrapped: Arc<Mutex<Vec<ChatResponse>>>,
}

impl SingleResponseStreamFactory for Factory {
    type Stream = Vec<ChatResponse>;

    fn wrap_single_response(&self, response: &ChatResponse) -> Self::Stream {
        self.events
            .lock()
            .unwrap()
            .push(Event::Wrapped(response.model.clone()));
        self.wrapped.lock().unwrap().push(response.clone());
        vec![response.clone()]
    }
}

pub(super) struct Fixture {
    pub(super) credentials: AccessToken,
    pub(super) messages: Vec<ChatMessage>,
    pub(super) optional_params: Object,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            credentials: AccessToken {
                token: SecretString::from(TOKEN),
                project: String::from("gemma-project"),
            },
            messages: serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#).unwrap(),
            optional_params: serde_json::from_str(r#"{"temperature":0.2}"#).unwrap(),
        }
    }

    pub(super) fn call<'text>(
        &'text self,
        model: &'text str,
        api_base: Option<&'text str>,
        stream: bool,
    ) -> GemmaCall<'text> {
        GemmaCall {
            model,
            messages: &self.messages,
            optional_params: &self.optional_params,
            api_base,
            credentials: &self.credentials,
            stream,
            invocation: Invocation::Async,
            timeout: None,
            identifier: Uuid::nil,
            clock: || 0,
        }
    }
}

pub(super) struct Run {
    pub(super) result: Result<GemmaReply<Vec<ChatResponse>>, FixtureError>,
    pub(super) caller_requests: Vec<SentRequest>,
    pub(super) pooled_requests: Vec<SentRequest>,
    pub(super) events: Vec<Event>,
    pub(super) wrapped: Vec<ChatResponse>,
}

pub(super) fn run(call: GemmaCall<'_>, reply: Reply, supplied: Option<SuppliedClientKind>) -> Run {
    let events = Arc::new(Mutex::new(Vec::new()));
    let caller_sent = Arc::new(Mutex::new(Vec::new()));
    let pooled_sent = Arc::new(Mutex::new(Vec::new()));
    let wrapped = Arc::new(Mutex::new(Vec::new()));
    let caller = FixtureClient {
        reply,
        sent: Arc::clone(&caller_sent),
        events: Arc::clone(&events),
    };
    let pooled = FixtureClient {
        reply,
        sent: Arc::clone(&pooled_sent),
        events: Arc::clone(&events),
    };
    let factory = Factory {
        events: Arc::clone(&events),
        wrapped: Arc::clone(&wrapped),
    };
    let supplied = supplied.map(|kind| SuppliedClient {
        kind,
        client: &caller,
    });
    let runtime = Builder::new_current_thread().build().unwrap();
    let result = runtime.block_on(completion(&call, supplied, &pooled, &factory));
    Run {
        result,
        caller_requests: caller_sent.lock().unwrap().clone(),
        pooled_requests: pooled_sent.lock().unwrap().clone(),
        events: events.lock().unwrap().clone(),
        wrapped: wrapped.lock().unwrap().clone(),
    }
}

pub(super) fn body_json(request: &SentRequest) -> Object {
    serde_json::from_slice(&request.body).unwrap()
}

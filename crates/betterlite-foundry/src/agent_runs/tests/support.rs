#![cfg(test)]

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::future::Future;
use std::iter::repeat_n;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::count::error::CountError;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;

use crate::agent_requests::prepare::{Headers, PreparedAgentCall, prepare};
use crate::agent_requests::request::AgentCallRequest;
use crate::agent_results::reply::{AgentReply, TokenCountRequest, TokenCounter};
use crate::agent_runs::dispatch::{DispatchCall, EntraResolver};
use crate::agent_runs::error::AgentRunError;
use crate::agent_runs::flow::{CompletionCall, completion};
use crate::agent_runs::transport::{
    AgentRunClientFactory, AgentRunMethod, AgentRunMode, AgentRunRequest, AgentRunTransport,
    Sleeper,
};
use crate::credentials::error::CredentialsError;

pub(super) const BASE: &str = "https://base.example/api/projects/p";
pub(super) const MODEL: &str = "azure_ai/agents/asst_1";
pub(super) const SECRET: &str = "fixture-secret-token";

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn messages(text: &str) -> Vec<Object> {
    serde_json::from_str(text).expect("a JSON message list")
}

pub(super) fn secret_headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| (String::from(*name), SecretString::from(*value)))
        .collect()
}

pub(super) fn shown(headers: &Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

pub(super) fn prepared(
    optional_params: &Object,
    api_key: Option<&SecretString>,
) -> PreparedAgentCall {
    let headers = secret_headers(&[("X-Trace", "trace-1")]);
    let request = AgentCallRequest {
        model: MODEL,
        messages: &[],
        optional_params,
        headers: &headers,
        api_key,
        api_base: Some(BASE),
    };
    prepare(&request).expect("a prepared call")
}

pub(super) static NO_ENVIRONMENT: BTreeMap<String, String> = BTreeMap::new();

pub(super) struct CallFixture {
    pub(super) api_base: &'static str,
    pub(super) optional_params: Object,
    pub(super) litellm_params: Object,
    pub(super) headers: Headers,
    pub(super) api_key: Option<SecretString>,
    pub(super) acompletion: bool,
    pub(super) stream: bool,
}

impl CallFixture {
    pub(super) fn new(api_base: &'static str) -> Self {
        Self {
            api_base,
            optional_params: Object::new(),
            litellm_params: Object::new(),
            headers: Headers::new(),
            api_key: None,
            acompletion: false,
            stream: false,
        }
    }

    pub(super) fn call(&self) -> DispatchCall<'_> {
        DispatchCall {
            model: MODEL,
            messages: &[],
            optional_params: &self.optional_params,
            litellm_params: &self.litellm_params,
            headers: &self.headers,
            environment: &NO_ENVIRONMENT,
            api_base: Some(self.api_base),
            api_key: self.api_key.as_ref(),
            reply: AgentReply::default(),
            acompletion: self.acompletion,
            stream: self.stream,
        }
    }
}

pub(super) fn block_on<Fut>(future: Fut) -> Fut::Output
where
    Fut: Future,
{
    let runtime = Builder::new_current_thread().build().expect("a runtime");
    runtime.block_on(future)
}

#[derive(Debug, Clone)]
pub(super) struct Step {
    pub status: u16,
    pub body: Vec<u8>,
}

#[must_use]
pub(super) fn step(status: u16, body: &str) -> Step {
    Step {
        status,
        body: body.as_bytes().to_vec(),
    }
}

#[must_use]
pub(super) fn repeated(count: usize, step: &Step) -> Vec<Step> {
    repeat_n(step.clone(), count).collect()
}

#[must_use]
pub(super) fn status_step(status: &str) -> Step {
    step(200, &format!(r#"{{"status": "{status}"}}"#))
}

#[must_use]
pub(super) fn failed_step(status: &str, message: &str) -> Step {
    step(
        200,
        &format!(r#"{{"status": "{status}", "last_error": {{"message": "{message}"}}}}"#),
    )
}

#[derive(Debug, Clone)]
pub(super) struct RecordedRequest {
    pub method: AgentRunMethod,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Vec<u8>>,
}

#[derive(Default)]
pub(super) struct TransportState {
    steps: VecDeque<Step>,
    requests: Vec<RecordedRequest>,
}

impl TransportState {
    pub(super) fn requests(&self) -> &[RecordedRequest] {
        &self.requests
    }

    pub(super) fn remaining(&self) -> usize {
        self.steps.len()
    }
}

#[derive(Default)]
pub(super) struct ScriptedTransport {
    state: Mutex<TransportState>,
}

#[must_use]
pub(super) fn transport(steps: Vec<Step>) -> Arc<ScriptedTransport> {
    let state = TransportState {
        steps: VecDeque::from(steps),
        requests: Vec::new(),
    };
    Arc::new(ScriptedTransport {
        state: Mutex::new(state),
    })
}

impl ScriptedTransport {
    pub(super) fn state(&self) -> MutexGuard<'_, TransportState> {
        self.state.lock().expect("the scripted transport")
    }
}

#[async_trait]
impl AgentRunTransport for Arc<ScriptedTransport> {
    async fn send(&self, request: AgentRunRequest) -> Result<UpstreamResponse, TransportError> {
        let mut state = self.state.lock().expect("the scripted transport");
        state.requests.push(RecordedRequest {
            method: request.method,
            url: request.url,
            headers: shown(&request.headers),
            body: request.body,
        });
        let next = state.steps.pop_front();
        drop(state);
        let Some(step) = next else {
            return Err(TransportError::Rejected(HttpError::Required {
                field_name: String::from("script step"),
            }));
        };
        Ok(UpstreamResponse {
            status: step.status,
            headers: HeaderMap::new(),
            body: step.body,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FactoryCall {
    pub kind: &'static str,
    pub ssl_verify: Json,
}

pub(super) struct RecordingFactory {
    transport: Arc<ScriptedTransport>,
    calls: Mutex<Vec<FactoryCall>>,
}

impl RecordingFactory {
    pub(super) fn new(transport: &Arc<ScriptedTransport>) -> Self {
        Self {
            transport: Arc::clone(transport),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn client(&self, kind: &'static str, ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FactoryCall {
                kind,
                ssl_verify: ssl_verify.clone(),
            });
        Box::new(Arc::clone(&self.transport))
    }

    pub(super) fn calls(&self) -> Vec<FactoryCall> {
        self.calls
            .lock()
            .expect("the recorded client kinds")
            .clone()
    }
}

impl AgentRunClientFactory for RecordingFactory {
    fn sync_client(&self, ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        self.client("sync", ssl_verify)
    }

    fn async_client(&self, ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        self.client("async", ssl_verify)
    }
}

pub(super) struct UnusedFactory;

impl AgentRunClientFactory for UnusedFactory {
    fn sync_client(&self, _ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        panic!("a supplied client must be reused for sync")
    }

    fn async_client(&self, _ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        panic!("a supplied client must be reused for async")
    }
}

#[derive(Default)]
pub(super) struct RecordingSleeper {
    waits: Mutex<Vec<Duration>>,
}

impl RecordingSleeper {
    pub(super) fn waits(&self) -> Vec<Duration> {
        self.waits.lock().expect("the recorded waits").clone()
    }
}

#[async_trait]
impl Sleeper for RecordingSleeper {
    async fn sleep(&self, duration: Duration) {
        self.waits
            .lock()
            .expect("the recorded waits")
            .push(duration);
    }
}

pub(super) struct FixedCounter;

#[async_trait]
impl TokenCounter for FixedCounter {
    async fn count_request(&self, request: &TokenCountRequest<'_>) -> Result<u64, CountError> {
        if let Some(messages) = request.messages {
            return Ok(u64::try_from(messages.len()).expect("a token count"));
        }
        Ok(u64::try_from(request.text.unwrap_or_default().len()).expect("a token count"))
    }
}

pub(super) struct RecordingResolver {
    token: Option<String>,
    params: Mutex<Vec<Object>>,
}

impl RecordingResolver {
    pub(super) fn new(token: Option<&str>) -> Self {
        Self {
            token: token.map(str::to_owned),
            params: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn params(&self) -> Vec<Object> {
        self.params.lock().expect("the recorded parameters").clone()
    }
}

impl EntraResolver for RecordingResolver {
    fn resolve(
        &self,
        params: &Object,
        _environment: &BTreeMap<String, String>,
    ) -> Result<Option<SecretString>, CredentialsError> {
        self.params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.clone());
        Ok(self.token.clone().map(SecretString::from))
    }
}

pub(super) struct FailingResolver;

impl EntraResolver for FailingResolver {
    fn resolve(
        &self,
        _params: &Object,
        _environment: &BTreeMap<String, String>,
    ) -> Result<Option<SecretString>, CredentialsError> {
        Err(CredentialsError::InvalidParam { field: "tenant_id" })
    }
}

pub(super) struct Run {
    pub messages: Vec<Object>,
    pub optional_params: Object,
    pub steps: Vec<Step>,
    pub mode: AgentRunMode,
    pub supplied: bool,
    pub key: Option<String>,
}

impl Run {
    pub(super) fn sync(steps: Vec<Step>) -> Self {
        Self {
            messages: Vec::new(),
            optional_params: Object::new(),
            steps,
            mode: AgentRunMode::Sync,
            supplied: true,
            key: Some(String::from(SECRET)),
        }
    }
}

pub(super) struct Outcome {
    pub result: Result<AgentReply, AgentRunError>,
    pub transport: Arc<ScriptedTransport>,
    pub sleeps: Vec<Duration>,
    pub factory_calls: Vec<FactoryCall>,
}

pub(super) fn finish(call: &CompletionCall<'_>, steps: Vec<Step>) -> Outcome {
    let scripted = transport(steps);
    let factory = RecordingFactory::new(&scripted);
    let sleeper = RecordingSleeper::default();
    let result = block_on(completion(call, &factory, &sleeper, &FixedCounter));
    Outcome {
        result,
        transport: scripted,
        sleeps: sleeper.waits(),
        factory_calls: factory.calls(),
    }
}

pub(super) fn execute(case: &Run) -> Outcome {
    let key = case.key.as_deref().map(SecretString::from);
    let prepared = prepared(&case.optional_params, key.as_ref());
    let scripted = transport(case.steps.clone());
    let supplied: Option<&dyn AgentRunTransport> = case.supplied.then_some(&scripted);
    let unused = UnusedFactory;
    let factory = RecordingFactory::new(&scripted);
    let sleeper = RecordingSleeper::default();
    let clients: &dyn AgentRunClientFactory = if case.supplied { &unused } else { &factory };
    let call = CompletionCall {
        model: MODEL,
        messages: &case.messages,
        optional_params: &case.optional_params,
        prepared,
        ssl_verify: Json::Null,
        reply: AgentReply::default(),
        mode: case.mode,
        supplied,
    };
    let result = block_on(completion(&call, clients, &sleeper, &FixedCounter));
    Outcome {
        result,
        transport: scripted,
        sleeps: sleeper.waits(),
        factory_calls: factory.calls(),
    }
}

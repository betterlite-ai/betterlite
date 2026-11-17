use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_foundry::agent_requests::prepare::Headers;
use betterlite_foundry::agent_results::reply::{TokenCountRequest, TokenCounter};
use betterlite_foundry::agent_runs::dispatch::EntraResolver;
use betterlite_foundry::agent_runs::transport::{
    AgentRunClientFactory, AgentRunMethod, AgentRunRequest, AgentRunTransport, Sleeper,
};
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::count::error::CountError;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};

use super::input::Step;

const WATCHED: [&str; 6] = [
    "azure_scope",
    "tenant_id",
    "client_id",
    "client_secret",
    "azure_ad_token",
    "api_key",
];

pub(super) fn secret_headers(headers: &BTreeMap<String, String>) -> Headers {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect()
}

pub(super) fn shown_headers(headers: &Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

#[derive(Clone)]
pub(super) struct ScriptedTransport {
    steps: Arc<Mutex<VecDeque<Step>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedTransport {
    pub(super) fn new(steps: &[Step]) -> Self {
        Self {
            steps: Arc::new(Mutex::new(steps.iter().cloned().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn requests(&self) -> Value {
        let views: Vec<Value> = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Value::Array(views)
    }
}

#[async_trait]
impl AgentRunTransport for ScriptedTransport {
    async fn send(&self, request: AgentRunRequest) -> Result<UpstreamResponse, TransportError> {
        let body = request
            .body
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
        let recorded = json!({
            "method": match request.method {
                AgentRunMethod::Get => "GET",
                AgentRunMethod::Post => "POST",
            },
            "url": request.url,
            "headers": shown_headers(&request.headers),
            "body": body,
            // the handler never passes a timeout to these calls, and neither does the flow
            "timeout": Value::Null,
        });
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        let popped = self
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(step) = popped else {
            return Err(TransportError::Rejected(HttpError::Required {
                field_name: String::from("scripted step"),
            }));
        };
        Ok(UpstreamResponse {
            status: step.status,
            headers: HeaderMap::new(),
            body: step.body.into_bytes(),
        })
    }
}

pub(super) struct RecordingFactory {
    pub(super) transport: ScriptedTransport,
    pub(super) kind: Mutex<&'static str>,
}

impl RecordingFactory {
    pub(super) fn new(transport: &ScriptedTransport) -> Self {
        Self {
            transport: transport.clone(),
            kind: Mutex::new(""),
        }
    }

    fn client(&self, kind: &'static str) -> Box<dyn AgentRunTransport> {
        *self.kind.lock().unwrap_or_else(PoisonError::into_inner) = kind;
        Box::new(self.transport.clone())
    }
}

impl AgentRunClientFactory for RecordingFactory {
    fn sync_client(&self, _ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        self.client("sync")
    }

    fn async_client(&self, _ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        self.client("async")
    }
}

#[derive(Default)]
pub(super) struct RecordingSleeper {
    waits: Mutex<Vec<f64>>,
}

impl RecordingSleeper {
    pub(super) fn values(&self) -> Vec<f64> {
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl Sleeper for RecordingSleeper {
    async fn sleep(&self, duration: Duration) {
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration.as_secs_f64());
    }
}

pub(super) struct CountingCounter;

#[async_trait]
impl TokenCounter for CountingCounter {
    async fn count_request(&self, request: &TokenCountRequest<'_>) -> Result<u64, CountError> {
        let count = match (request.messages, request.text) {
            (Some(messages), _) => messages.len(),
            (None, Some(text)) => text.chars().count(),
            (None, None) => 0,
        };
        u64::try_from(count).map_err(|_count| CountError::Overflow)
    }
}

pub(super) struct RecordingResolver {
    token: Option<String>,
    seen: Mutex<Vec<Value>>,
}

impl RecordingResolver {
    pub(super) fn new(token: Option<&str>) -> Self {
        Self {
            token: token.map(str::to_owned),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn watched(params: &Object) -> Value {
    let mut watched = Map::new();
    for name in WATCHED {
        let value = params.get(name).map_or(Value::Null, |value| {
            serde_json::to_value(value).unwrap_or(Value::Null)
        });
        watched.insert(String::from(name), value);
    }
    Value::Object(watched)
}

impl EntraResolver for RecordingResolver {
    fn resolve(
        &self,
        params: &Object,
        _environment: &BTreeMap<String, String>,
    ) -> Result<Option<SecretString>, CredentialsError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(watched(params));
        Ok(self.token.clone().map(SecretString::from))
    }
}

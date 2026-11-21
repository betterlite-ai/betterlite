use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_foundry::agent_results::reply::{TokenCountRequest, TokenCounter};
use betterlite_foundry::agent_runs::dispatch::EntraResolver;
use betterlite_foundry::agent_runs::transport::{
    AgentRunClientFactory, AgentRunMethod, AgentRunRequest, AgentRunTransport, Sleeper,
};
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::count::error::CountError;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};

#[derive(Clone)]
pub struct Step {
    pub status: u16,
    pub body: String,
}

impl Step {
    pub fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sent {
    pub method: AgentRunMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

// One scripted conversation: each request takes the next answer and is kept for the case to compare
pub struct Scripted {
    steps: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<Sent>>,
}

impl Scripted {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Mutex::new(VecDeque::from(steps)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<Sent> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AgentRunTransport for Arc<Scripted> {
    async fn send(&self, request: AgentRunRequest) -> Result<UpstreamResponse, TransportError> {
        let body = request
            .body
            .as_deref()
            .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap_or_default());
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                method: request.method,
                url: request.url,
                headers: request
                    .headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                    .collect(),
                body,
            });
        let step = self
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Step::new(500, "{}"));
        Ok(UpstreamResponse {
            status: step.status,
            headers: HeaderMap::new(),
            body: step.body.into_bytes(),
        })
    }
}

// B173 builds the real clients; this factory hands the same script to either kind
pub struct Factory {
    pub script: Arc<Scripted>,
}

impl AgentRunClientFactory for Factory {
    fn sync_client(&self, _ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        Box::new(Arc::clone(&self.script))
    }

    fn async_client(&self, _ssl_verify: &Json) -> Box<dyn AgentRunTransport> {
        Box::new(Arc::clone(&self.script))
    }
}

#[derive(Default)]
pub struct Clock {
    pub waits: Mutex<Vec<Duration>>,
}

#[async_trait]
impl Sleeper for Clock {
    async fn sleep(&self, duration: Duration) {
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
    }
}

#[derive(Default)]
pub struct Tally;

#[async_trait]
impl TokenCounter for Tally {
    async fn count_request(&self, request: &TokenCountRequest<'_>) -> Result<u64, CountError> {
        if let Some(messages) = request.messages {
            return Ok(u64::try_from(messages.len()).unwrap_or(u64::MAX));
        }
        Ok(u64::try_from(request.text.map_or(0, str::len)).unwrap_or(u64::MAX))
    }
}

// The Azure AD lookup main.py's agents branch reaches; a fixed token keeps the case off the network
pub struct Resolver {
    pub token: Option<String>,
    pub seen: Mutex<Vec<Object>>,
}

impl EntraResolver for Resolver {
    fn resolve(
        &self,
        params: &Object,
        _environment: &BTreeMap<String, String>,
    ) -> Result<Option<SecretString>, CredentialsError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.clone());
        Ok(self.token.clone().map(SecretString::from))
    }
}

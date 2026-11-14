use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use futures_util::stream;
use secrecy::SecretString;
use thiserror::Error;
use uuid::Uuid;

use crate::agentcore_transform::SignedRequest;
use crate::agentcore_transform::params::LitellmParams;
use crate::agentcore_transform::runtime::{
    AgentCoreRuntime, HttpRequest, HttpResponse, SigningRequest, StreamedResponse,
};
use crate::completion_bridge::EventSource;
use crate::completion_bridge::wire::{A2AMessage, MessageSendParams, Role};

pub(crate) const CONTEXT_ID: &str = "0123456789abcdef0123456789abcdef";
pub(crate) const GENERATED: Uuid = Uuid::from_u128(7);
pub(crate) const SESSION_PREFIX: &str = "litellm-session";
pub(crate) const URL: &str =
    "https://bedrock-agentcore.us-west-2.amazonaws.com/runtimes/agentcore/arn/invocations/";

#[derive(Debug, Error)]
#[error("{0}")]
pub(crate) struct FixtureError(pub(crate) &'static str);

pub(crate) struct Source;

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        GENERATED
    }

    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}

pub(crate) struct Signing {
    pub(crate) model: String,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) headers: BTreeMap<String, String>,
}

pub(crate) struct Posted {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct FixtureRuntime {
    pub(crate) url: String,
    pub(crate) url_error: Option<&'static str>,
    pub(crate) user: Option<String>,
    pub(crate) user_error: Option<&'static str>,
    pub(crate) session_error: Option<&'static str>,
    pub(crate) signing_error: Option<&'static str>,
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    pub(crate) content_type: String,
    pub(crate) lines: Vec<String>,
    pub(crate) post_error: Option<&'static str>,
    pub(crate) signing: Mutex<Vec<Signing>>,
    pub(crate) posted: Mutex<Vec<Posted>>,
}

pub(crate) fn fixture() -> FixtureRuntime {
    FixtureRuntime {
        url: URL.to_owned(),
        url_error: None,
        user: None,
        user_error: None,
        session_error: None,
        signing_error: None,
        status: 200,
        body: Vec::new(),
        content_type: "application/json".to_owned(),
        lines: Vec::new(),
        post_error: None,
        signing: Mutex::new(Vec::new()),
        posted: Mutex::new(Vec::new()),
    }
}

impl FixtureRuntime {
    fn response(&self) -> HttpResponse {
        HttpResponse {
            status: self.status,
            body: self.body.clone(),
        }
    }

    fn record(&self, request: &HttpRequest<'_>) {
        self.posted
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(Posted {
                url: request.url.to_owned(),
                headers: request.headers.clone(),
                body: request.body.to_vec(),
            });
    }
}

type Params = BTreeMap<String, AgentJson>;

impl AgentCoreRuntime for FixtureRuntime {
    type Error = FixtureError;
    type Lines = stream::Iter<std::vec::IntoIter<Result<String, FixtureError>>>;

    fn complete_url(&self, model: &str, _optional_params: &Params) -> Result<String, Self::Error> {
        match self.url_error {
            Some(message) => Err(FixtureError(message)),
            None => Ok(format!("{}{model}", self.url)),
        }
    }

    fn session_id(
        &self,
        _optional_params: &Params,
        generated: Uuid,
    ) -> Result<String, Self::Error> {
        match self.session_error {
            Some(message) => Err(FixtureError(message)),
            None => Ok(format!("{SESSION_PREFIX}-{generated}")),
        }
    }

    fn user_id(&self, _optional_params: &Params) -> Result<Option<String>, Self::Error> {
        if let Some(message) = self.user_error {
            return Err(FixtureError(message));
        }
        Ok(self.user.clone())
    }

    async fn sign(&self, request: &SigningRequest<'_>) -> Result<SignedRequest, Self::Error> {
        let body = serde_json::to_vec(request.request_data).expect("the envelope encodes");
        self.signing
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(Signing {
                model: request.model.to_owned(),
                api_key: request.api_key.cloned(),
                headers: request.headers.clone(),
            });
        match self.signing_error {
            Some(message) => Err(FixtureError(message)),
            None => Ok(SignedRequest {
                url: request.api_base.to_owned(),
                headers: request.headers.clone(),
                body,
            }),
        }
    }

    async fn post(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, Self::Error> {
        self.record(request);
        self.post_error
            .map_or_else(|| Ok(self.response()), |message| Err(FixtureError(message)))
    }

    async fn post_stream(
        &self,
        request: &HttpRequest<'_>,
    ) -> Result<StreamedResponse<Self::Lines>, Self::Error> {
        self.record(request);
        if let Some(message) = self.post_error {
            return Err(FixtureError(message));
        }
        let lines: Vec<Result<String, FixtureError>> = self.lines.iter().cloned().map(Ok).collect();
        Ok(StreamedResponse {
            status: self.status,
            content_type: self.content_type.clone(),
            lines: stream::iter(lines),
        })
    }
}

pub(crate) fn params(context_id: Option<AgentJson>) -> MessageSendParams {
    let mut extra = BTreeMap::new();
    if let Some(value) = context_id {
        extra.insert("contextId".to_owned(), value);
    }
    MessageSendParams {
        message: A2AMessage {
            role: Role("user".to_owned()),
            parts: vec![serde_json::json!({"kind": "text", "text": "hello"})],
            message_id: Some("msg-1".to_owned()),
            metadata: None,
            extra,
        },
        metadata: None,
        extra: BTreeMap::new(),
    }
}

pub(crate) fn litellm_params(entries: &[(&str, AgentJson)]) -> LitellmParams {
    let raw: BTreeMap<String, AgentJson> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    LitellmParams::parse(&raw).expect("the fixture litellm_params parse")
}

pub(crate) fn bearer_params() -> LitellmParams {
    litellm_params(&[
        ("model", serde_json::json!("bedrock/agentcore/arn")),
        ("api_key", serde_json::json!("jwt-token")),
        ("litellm_a2a_user_api_key_hash", serde_json::json!("key-2")),
    ])
}

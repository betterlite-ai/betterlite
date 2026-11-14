use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, pending, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use secrecy::SecretString;
use tokio::time::sleep;

use crate::cache_prediction_targets::counting::TokenCounter;
use crate::count_tokens::handler::CountTokensTransport;
use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings};

pub(crate) const MODEL: &str = "claude-x";
pub(crate) const KEY: &str = "sk-ant-api03-test";

pub(crate) fn settings(api_base: &str, api_key: Option<&str>) -> AnthropicSettings {
    AnthropicSettings {
        api_key: api_key.map(SecretString::from),
        auth_token: None,
        api_base: api_base.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn params(json: &str) -> JsonObject {
    serde_json::from_str(json).expect("deployment params")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json")
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            HeaderValue::from_str(value).expect("header value"),
        );
    }
    map
}

pub(crate) struct Resolution {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolution {
    pub(crate) fn new() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Sent {
    pub(crate) method: Method,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) deadline: Duration,
}

impl Sent {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(present, _)| present == name)
            .map(|(_, value)| value.as_str())
    }

    fn read(request: OutboundRequest) -> Self {
        let OutboundRequest {
            method,
            url,
            headers,
            body,
            deadline,
            ..
        } = request;
        let payload = match body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        let pairs = headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        Self {
            method,
            url: url.to_string(),
            headers: pairs,
            body: payload,
            deadline,
        }
    }
}

type Replies = Mutex<VecDeque<Option<(u16, Vec<u8>)>>>;

pub(crate) struct Script {
    replies: Replies,
    sent: Mutex<Vec<Sent>>,
}

impl Script {
    pub(crate) fn replying(replies: &[(u16, &[u8])]) -> Self {
        let scripted = replies
            .iter()
            .map(|(status, payload)| Some((*status, payload.to_vec())))
            .collect();
        Self {
            replies: Mutex::new(scripted),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            replies: Mutex::new(VecDeque::from([None])),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn request(&self) -> Sent {
        let mut requests = self.requests();
        assert_eq!(requests.len(), 1);
        requests.remove(0)
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent::read(request));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .flatten();
        ready(
            reply
                .ok_or(TransportError::Closed)
                .map(|(status, payload)| UpstreamResponse {
                    status,
                    headers: HeaderMap::new(),
                    body: payload,
                }),
        )
    }
}

pub(crate) struct Pending;

impl CountTokensTransport for Pending {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        pending::<Result<UpstreamResponse, TransportError>>()
    }
}

pub(crate) struct Slow(pub(crate) Duration);

impl CountTokensTransport for Slow {
    async fn send(&self, _request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        sleep(self.0).await;
        Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: br#"{"input_tokens":7}"#.to_vec(),
        })
    }
}

pub(crate) struct ScriptedCounter {
    replies: Mutex<VecDeque<Option<i64>>>,
    bodies: Mutex<Vec<Json>>,
}

impl ScriptedCounter {
    pub(crate) fn new(replies: &[Option<i64>]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().copied().collect()),
            bodies: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn counted_bodies(&self) -> Vec<Json> {
        self.bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TokenCounter for ScriptedCounter {
    fn count(
        &self,
        _model: &str,
        _api_key: &SecretString,
        body: &Json,
    ) -> impl Future<Output = Option<i64>> + Send {
        self.bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(body.clone());
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .flatten();
        ready(reply)
    }
}

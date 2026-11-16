use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_config::sections::Section;
use betterlite_config::sections::anthropic_count_tokens::Raw as BetaRaw;
use betterlite_config::sections::azure_credentials::{DeploymentSettings, Raw as CredentialRaw};
use betterlite_config::sections::azure_url_policy::Raw as KeyRaw;
use betterlite_config::sections::foundry_credentials;
use betterlite_config::sections::routing_timeouts::Settings as TimeoutSettings;
use betterlite_foundry::anthropic_messages::environment::ParamsKey;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use super::input::{Globals, Reply, failure};

use crate::helpers::HelperError;

pub(crate) const COUNT_HEADERS: [&str; 6] = [
    "content-type",
    "x-api-key",
    "api-key",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
];

#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
    pub(crate) timeout: f64,
}

pub(crate) struct Script {
    reply: Option<(u16, Vec<u8>)>,
    sent: Mutex<Vec<Seen>>,
}

impl Debug for Script {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Script").finish_non_exhaustive()
    }
}

impl Script {
    pub(crate) fn new(reply: Option<&Reply>) -> Self {
        Self {
            reply: reply.map(|found| (found.status, found.body.clone().into_bytes())),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Seen> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: OutboundRequest) {
        let OutboundRequest {
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
            .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
            .collect();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                url: url.to_string(),
                headers: pairs,
                body: payload,
                timeout: deadline.as_secs_f64(),
            });
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        ready(
            self.reply
                .clone()
                .ok_or(TransportError::Closed)
                .map(|(status, payload)| UpstreamResponse {
                    status,
                    headers: HeaderMap::new(),
                    body: payload,
                }),
        )
    }
}

pub(crate) fn request_json(seen: &Seen) -> Value {
    let body = serde_json::from_slice::<Value>(&seen.body).unwrap_or(Value::Null);
    json!({
        "url": seen.url,
        "timeout": seen.timeout,
        "headers": filtered_headers(&seen.headers),
        "body": body,
    })
}

fn filtered_headers(headers: &BTreeMap<String, String>) -> Value {
    let mut fields = serde_json::Map::new();
    for name in COUNT_HEADERS {
        let value = headers
            .get(name)
            .map_or(Value::Null, |found| Value::String(found.clone()));
        fields.insert(name.to_owned(), value);
    }
    Value::Object(fields)
}

pub(crate) fn request_timeout(override_seconds: Option<f64>) -> Result<Duration, Box<HelperError>> {
    let seconds = override_seconds.unwrap_or_else(|| {
        let settings = TimeoutSettings::default();
        settings.global.seconds
    });
    Duration::try_from_secs_f64(seconds).map_err(failure)
}

pub(crate) fn beta_version(
    environment: &BTreeMap<String, String>,
) -> Result<String, Box<HelperError>> {
    let raw = BetaRaw::default();
    let settings = raw.convert(environment).map_err(failure)?;
    Ok(settings.beta_version)
}

// the B2200 credential chain over the deployment params and the environment
pub(crate) fn credential_settings(
    litellm_params: &BTreeMap<String, Option<String>>,
    environment: &BTreeMap<String, String>,
) -> Result<DeploymentSettings, Box<HelperError>> {
    let source = json!({
        "litellm_settings": {"enable_azure_ad_token_refresh": false},
        "model_list": [{"litellm_params": litellm_params}],
    });
    let raw: CredentialRaw = serde_json::from_value(source).map_err(failure)?;
    let resolved = raw.convert(environment).map_err(failure)?;
    resolved
        .deployments
        .into_iter()
        .next()
        .ok_or_else(|| failure("no deployment resolved"))
}

// AZURE_AI_API_KEY and AZURE_AI_API_BASE, resolved by the shared foundry credential loader
pub(crate) fn foundry_settings(
    environment: &BTreeMap<String, String>,
) -> foundry_credentials::Settings {
    foundry_credentials::Settings::resolve_environment(environment)
}

// litellm.api_key, litellm.azure_key and the Azure key variables, through the production config section
pub(crate) fn configured_key(
    globals: &Globals,
    environment: &BTreeMap<String, String>,
) -> Result<Option<SecretString>, Box<HelperError>> {
    let source = json!({
        "litellm_settings": {"api_key": globals.api_key, "azure_key": globals.azure_key},
        "model_list": [{"litellm_params": {}}],
    });
    let raw: KeyRaw = serde_json::from_value(source).map_err(failure)?;
    let resolved = raw.convert(environment).map_err(failure)?;
    Ok(resolved
        .deployments
        .into_iter()
        .next()
        .and_then(|deployment| deployment.api_key))
}

// litellm_params keeps the dict LiteLLM received: a null api_key is present, a missing one is absent
pub(crate) fn key_shape<'data>(
    litellm_params: &'data BTreeMap<String, Option<String>>,
    secret: Option<&'data SecretString>,
) -> ParamsKey<'data> {
    match (litellm_params.get("api_key"), secret) {
        (None, _) => ParamsKey::DictAbsent,
        (Some(_), None) => ParamsKey::DictNull,
        (Some(_), Some(value)) => ParamsKey::Dict(value),
    }
}

pub(crate) fn params_secret(
    litellm_params: &BTreeMap<String, Option<String>>,
) -> Option<SecretString> {
    litellm_params
        .get("api_key")
        .and_then(|entry| entry.as_deref())
        .map(SecretString::from)
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Logs {
    pub(crate) fn entries(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|message| warning_entry(message))
            .collect()
    }
}

fn warning_entry(message: &str) -> Value {
    let (subject, detail) = message.split_once(": ").unwrap_or((message, ""));
    let status = detail
        .strip_prefix("status=")
        .and_then(|rest| rest.split(',').next())
        .and_then(|number| number.parse::<u16>().ok());
    json!({"subject": subject, "status": status})
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

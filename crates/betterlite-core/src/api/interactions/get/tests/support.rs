use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{
    ErrorClasses, Logging, Params, PreparedRequest, RequestHeaders, ResponseHeaders,
    UpstreamResponse,
};
use betterlite_http::interactions::get::{
    EnvironmentRequest, GetInteractionConfig, InteractionIdRequest,
};
use betterlite_interactions::types::response::InteractionsApiResponse;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use crate::api::agents::item::error::ProviderFailure;

use super::super::{InteractionGetDeps, InteractionGetRequest};

pub(super) const GET_BODY: &str = r#"{"id": "int_1", "model": "gemini-2.5-flash"}"#;

#[derive(Clone, Debug)]
pub(super) enum Seen {
    Environment {
        params: Params,
        headers: Vec<(String, String)>,
    },
    Request {
        interaction_id: String,
        headers: Vec<(String, String)>,
    },
    Response {
        status: u16,
        input: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Environment,
    Request,
    Response,
}

impl Seen {
    const fn kind(&self) -> Kind {
        match self {
            Self::Environment { .. } => Kind::Environment,
            Self::Request { .. } => Kind::Request,
            Self::Response { .. } => Kind::Response,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
    #[error("the body is not JSON")]
    Body(#[source] serde_json::Error),
    #[error("{message}")]
    Refused { status: u16, message: String },
}

impl ProviderFailure for FixtureError {
    fn status_code(&self) -> u16 {
        match self {
            Self::Body(_) => 500,
            Self::Refused { status, .. } => *status,
        }
    }

    fn message(&self) -> String {
        self.to_string()
    }
}

// the Gemini interactions config's shape, with every transform's output pinned by the test
pub(super) struct Config {
    calls: Mutex<Vec<Seen>>,
    headers: RequestHeaders,
}

#[must_use]
pub(super) fn fixture() -> Config {
    Config {
        calls: Mutex::new(Vec::new()),
        headers: RequestHeaders::new(),
    }
}

impl Config {
    pub(super) fn calls(&self) -> Vec<Seen> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, seen: Seen) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
    }
}

impl ErrorClasses for Config {
    type Error = FixtureError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: &ResponseHeaders,
    ) -> FixtureError {
        FixtureError::Refused {
            status: status_code,
            message: message.to_owned(),
        }
    }
}

impl GetInteractionConfig for Config {
    type Output = InteractionsApiResponse;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, FixtureError> {
        self.record(Seen::Environment {
            params: request
                .litellm_params
                .map_or_else(Params::new, Clone::clone),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect(),
        });
        Ok(self.headers.clone())
    }

    fn transform_get_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        self.record(Seen::Request {
            interaction_id: request.interaction_id.to_owned(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect(),
        });
        Ok(PreparedRequest {
            url: format!(
                "{}/v1beta/interactions/{}",
                request.api_base, request.interaction_id
            ),
            params: Params::new(),
        })
    }

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        logging: &mut Logging,
    ) -> Result<InteractionsApiResponse, FixtureError> {
        let input = logging
            .pre_call
            .as_ref()
            .map_or_else(String::new, |call| call.input.clone());
        self.record(Seen::Response {
            status: response.status_code,
            input,
        });
        if !(200..300).contains(&response.status_code) {
            return Err(FixtureError::Refused {
                status: response.status_code,
                message: String::from_utf8(response.body.clone()).unwrap_or_default(),
            });
        }
        serde_json::from_slice(&response.body).map_err(FixtureError::Body)
    }
}

pub(super) struct Harness {
    clients: HttpClients,
    pub(super) state: LoggingState,
    pub(super) record: Logging,
    pub(super) redactor: Redactor,
}

impl Harness {
    #[must_use]
    pub(super) fn new() -> Self {
        let mut context = CorrelationContext::default();
        let redactor =
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles");
        let state = LoggingState::new(
            LoggingInit {
                model: String::new(),
                call_type: String::new(),
                start_time: moment(),
                litellm_call_id: "call-1".to_owned(),
                function_id: String::new(),
                messages: None,
                stream: None,
                litellm_trace_id: None,
                kwargs: IndexMap::new(),
                litellm_params: IndexMap::new(),
                callback_dynamic_params: IndexMap::new(),
                trusted_callback_vars: Vec::new(),
                standard_built_in_tools_params: IndexMap::new(),
                dynamic_callbacks: DynamicCallbacks::default(),
                applied_guardrails: None,
                request_logging: RequestLoggingFlags::default(),
                callback_policy: CallbackPolicy::default(),
            },
            CorrelationSetup {
                context: &mut context,
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated-trace"),
            Duration::default,
        )
        .expect("the logging fixture builds");
        Self {
            clients: HttpClients::new(policy(), proxy()),
            state,
            record: Logging::default(),
            redactor,
        }
    }

    pub(super) fn deps(&mut self) -> InteractionGetDeps<'_> {
        InteractionGetDeps {
            clients: &mut self.clients,
            logging: &mut self.state,
            http: &mut self.record,
            redactor: &self.redactor,
        }
    }
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

fn policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test/0".to_owned(),
    }
}

fn proxy() -> Arc<ProxyPolicy> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap())
}

pub(super) struct Call<'call> {
    interaction_id: &'call str,
    provider: Option<&'call str>,
    headers: BTreeMap<String, String>,
    kwargs: IndexMap<String, Json>,
}

impl<'call> Call<'call> {
    #[must_use]
    pub(super) fn new(interaction_id: &'call str) -> Self {
        Self {
            interaction_id,
            provider: None,
            headers: BTreeMap::new(),
            kwargs: IndexMap::new(),
        }
    }

    #[must_use]
    pub(super) fn provider(mut self, provider: &'call str) -> Self {
        self.provider = Some(provider);
        self
    }

    #[must_use]
    pub(super) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_owned(), value.to_owned());
        self
    }

    #[must_use]
    pub(super) fn kwarg(mut self, name: &str, value: Json) -> Self {
        self.kwargs.insert(name.to_owned(), value);
        self
    }

    pub(super) fn request(&self) -> InteractionGetRequest<'_> {
        InteractionGetRequest {
            interaction_id: self.interaction_id,
            custom_llm_provider: self.provider,
            extra_headers: self.headers.clone(),
            timeout: None,
            kwargs: &self.kwargs,
        }
    }
}

pub(super) fn request_line(head: &str) -> &str {
    head.lines().next().unwrap_or_default()
}

pub(super) fn base(address: SocketAddr) -> String {
    format!("http://{address}")
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn last(config: &Config, kind: Kind) -> Seen {
    let calls = config.calls();
    let found = calls.iter().rev().find(|seen| seen.kind() == kind);
    let Some(seen) = found else {
        panic!("no {kind:?} was recorded");
    };
    seen.clone()
}

pub(super) fn environment_params(config: &Config) -> Params {
    let Seen::Environment { params, .. } = last(config, Kind::Environment) else {
        panic!("the environment is validated");
    };
    params
}

pub(super) fn environment_headers(config: &Config) -> Vec<(String, String)> {
    let Seen::Environment { headers, .. } = last(config, Kind::Environment) else {
        panic!("the environment is validated");
    };
    headers
}

pub(super) fn request_of(config: &Config) -> (String, Vec<(String, String)>) {
    let Seen::Request {
        interaction_id,
        headers,
    } = last(config, Kind::Request)
    else {
        panic!("the request is transformed");
    };
    (interaction_id, headers)
}

pub(super) fn response_of(config: &Config) -> (u16, String) {
    let Seen::Response { status, input } = last(config, Kind::Response) else {
        panic!("the response is transformed");
    };
    (status, input)
}

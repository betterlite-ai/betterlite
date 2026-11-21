use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::header::USER_AGENT;
use http::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::auth_errors::conversion::{AuthFailure, ClientFailure};
use crate::auth_errors::handler::{
    AuthErrorHandler, AuthErrorSettings, FailedRequest, HandlerError, LogSwitches, RaisedFailure,
};
use crate::auth_errors::ports::{
    AuthErrorPorts, AuthLog, FailureHookCall, HookError, ServiceFailure,
};
use crate::auth_request_policy::ip::RequestProvenance;
use crate::auth_route_patterns::normalize::RouteNormalizer;
use crate::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use crate::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};

pub(super) const CLIENT_IP: &str = "203.0.113.7";
pub(super) const RESPONSE_ROUTE: &str = "/v1/responses/resp_123";
const PROVENANCE: RequestProvenance<'static> = RequestProvenance {
    forwarded_for: None,
    client_host: Some(CLIENT_IP),
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Event {
    Service {
        call_type: &'static str,
        duration: Duration,
    },
    Log(AuthLog),
    Seed {
        route: Option<String>,
        api_key: Option<String>,
        model: Option<Json>,
        has_span: bool,
    },
    Hook {
        request_data: JsonObject,
        original: AuthFailure,
        error_type: ProxyErrorTypes,
        route: String,
        user_id: Option<String>,
    },
}

pub(super) enum Reply {
    Original,
    Replace(AuthFailure),
    Raise,
}

#[derive(Default)]
pub(super) struct Resolver {
    pub(super) calls: Mutex<Vec<String>>,
}

impl ProviderResolution for Resolver {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        self.calls.lock().unwrap().push(model.to_owned());
        Ok(ResolvedModel {
            model: model.to_owned(),
            provider: ProviderIdentity::Known(ProviderId::Openai),
            api_base: None,
            api_key: None,
        })
    }
}

pub(super) struct Recorder {
    events: Mutex<Vec<Event>>,
    reply: Reply,
    pub(super) resolver: Resolver,
}

impl Recorder {
    pub(super) fn replying(reply: Reply) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            reply,
            resolver: Resolver::default(),
        }
    }

    fn push(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }

    pub(super) fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    pub(super) fn logs(&self) -> Vec<AuthLog> {
        let mut logs = Vec::new();
        for event in self.events() {
            if let Event::Log(entry) = event {
                logs.push(entry);
            }
        }
        logs
    }

    pub(super) fn hook_calls(&self) -> Vec<(JsonObject, AuthFailure, ProxyErrorTypes, String)> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Hook {
                    request_data,
                    original,
                    error_type,
                    route,
                    ..
                } => Some((request_data, original, error_type, route)),
                Event::Service { .. } | Event::Log(_) | Event::Seed { .. } => None,
            })
            .collect()
    }
}

#[async_trait]
impl AuthErrorPorts for Recorder {
    fn record_service_failure(&self, failure: &ServiceFailure<'_>) {
        self.push(Event::Service {
            call_type: failure.call_type,
            duration: failure.duration,
        });
    }

    fn log(&self, entry: AuthLog) {
        self.push(Event::Log(entry));
    }

    fn seed_identity(&self, identity: &UserAPIKeyAuth, model: Option<&Json>) {
        self.push(Event::Seed {
            route: identity.extra.request_route.clone(),
            api_key: identity
                .extra
                .api_key
                .0
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            model: model.cloned(),
            has_span: identity.extra.parent_otel_span.is_some(),
        });
    }

    fn native_resolution(&self) -> &dyn ProviderResolution {
        &self.resolver
    }

    fn router(&self) -> Option<&dyn DeploymentSnapshot> {
        None
    }

    async fn post_call_failure_hook(
        &self,
        call: FailureHookCall<'_>,
    ) -> Result<Option<AuthFailure>, HookError> {
        self.push(Event::Hook {
            request_data: call.request_data.clone(),
            original: call.original.clone(),
            error_type: call.error_type,
            route: call.route.to_owned(),
            user_id: call.identity.view.token.user_id.clone(),
        });
        match &self.reply {
            Reply::Original => Ok(None),
            Reply::Replace(failure) => Ok(Some(failure.clone())),
            Reply::Raise => Err(HookError::Raised),
        }
    }
}

pub(super) struct Fixture {
    api_key: SecretString,
    pub(super) request_data: JsonObject,
    headers: HeaderMap,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            api_key: SecretString::from("sk-presented".to_owned()),
            request_data: object(&[("model", Json::Text("gpt-4o".to_owned()))]),
            headers: HeaderMap::from_iter([(USER_AGENT, HeaderValue::from_static("curl/8.5"))]),
        }
    }

    pub(super) fn request(&self) -> FailedRequest<'_> {
        FailedRequest {
            request_data: &self.request_data,
            route: RESPONSE_ROUTE,
            parent_span: Some(ParentOtelSpan::Json(JsonData::String("server".to_owned()))),
            api_key: &self.api_key,
            resolved_identity: None,
            provenance: PROVENANCE,
            scope_headers: Some(&self.headers),
        }
    }
}

pub(super) async fn handle(
    ports: &Recorder,
    settings: AuthErrorSettings,
    error: AuthFailure,
    request: FailedRequest<'_>,
) -> Result<UserAPIKeyAuth, HandlerError> {
    let handler = AuthErrorHandler {
        ports,
        settings,
        routes: &RouteNormalizer::new().unwrap(),
        redactor: &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    handler.handle(error, request).await
}

pub(super) fn object(fields: &[(&str, Json)]) -> JsonObject {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) const fn settings(tracebacks: bool) -> AuthErrorSettings {
    AuthErrorSettings {
        log: LogSwitches {
            client_error_tracebacks: tracebacks,
            key_identity: false,
        },
        allow_on_outage: false,
        use_x_forwarded_for: false,
    }
}

pub(super) const OUTAGE_ALLOWED: AuthErrorSettings = AuthErrorSettings {
    log: LogSwitches {
        client_error_tracebacks: false,
        key_identity: false,
    },
    allow_on_outage: true,
    use_x_forwarded_for: false,
};

pub(super) fn http(status_code: u16, invalid_virtual_key: bool) -> AuthFailure {
    AuthFailure::Http {
        detail: "Invalid proxy server token passed.".to_owned(),
        param: None,
        status_code,
        invalid_virtual_key,
    }
}

pub(super) fn proxy(invalid_virtual_key: bool) -> AuthFailure {
    let exception = ProxyException::new(ProxyExceptionInit {
        message: "Invalid proxy server token passed.".to_owned(),
        error_type: "auth_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(401_i64),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    });
    AuthFailure::Proxy {
        exception,
        invalid_virtual_key,
    }
}

pub(super) fn database(error_type: ErrorType, message: &str) -> AuthFailure {
    AuthFailure::Database(DbError {
        error_type,
        message: message.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    })
}

pub(super) fn rejected(outcome: Result<UserAPIKeyAuth, HandlerError>) -> RaisedFailure {
    match outcome {
        Err(HandlerError::Rejected(raised)) => *raised,
        Ok(_) | Err(HandlerError::Hook(_) | HandlerError::Identity(_)) => {
            panic!("the handler raises the converted failure")
        }
    }
}

pub(super) fn wire(failure: &ClientFailure) -> (String, Option<String>, String) {
    let exception = failure.exception();
    (
        exception.error_type.clone(),
        exception.param.clone(),
        exception.code.clone(),
    )
}

pub(super) fn identity_with(fields: &[(&str, &str)]) -> UserAPIKeyAuth {
    let mut identity = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap();
    for (name, value) in fields {
        let text = Some((*value).to_owned());
        match *name {
            "key_alias" => identity.view.token.key_alias = text,
            "user_id" => identity.view.token.user_id = text,
            "user_email" => identity.extra.user_email = text,
            "team_id" => identity.view.token.team_id = text,
            _ => identity.view.team_alias = text,
        }
    }
    identity
}

pub(super) fn seeded(ports: &Recorder) -> (Option<String>, Option<String>, Option<Json>, bool) {
    let seeds: Vec<(Option<String>, Option<String>, Option<Json>, bool)> = ports
        .events()
        .into_iter()
        .filter_map(|event| match event {
            Event::Seed {
                route,
                api_key,
                model,
                has_span,
            } => Some((route, api_key, model, has_span)),
            Event::Service { .. } | Event::Log(_) | Event::Hook { .. } => None,
        })
        .collect();
    assert_eq!(seeds.len(), 1);
    seeds.into_iter().next().unwrap()
}

pub(super) fn budget(llm_provider: &str) -> AuthFailure {
    AuthFailure::BudgetExceeded {
        message: "Budget has been exceeded!".to_owned(),
        status_code: Some(422),
        llm_provider: llm_provider.to_owned(),
    }
}

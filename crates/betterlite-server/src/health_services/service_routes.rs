use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Extension, RawQuery, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::{Json, Router};
use betterlite_auth::auth_route_patterns::canonical::{
    RequestRouteSource, ScopeFields, get_request_route,
};
use betterlite_config::sections::health_services::Settings;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_logging::exporter_options::health::HealthState;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_resource_grants::denial_response;
use crate::auth::route_match::decoded_route_path;
use crate::failure_hook::ports::Clock;
use crate::health_services::alert_tests::{AlertActions, AlertSetup};
use crate::health_services::mock_requests::MockRequests;
use crate::health_services::service_name::{ServiceName, unknown_service_message};
use crate::health_services::service_probes::{
    CallbackSource, Caller, IntegrationChecks, Probes, ServiceError, dispatch,
};
use crate::list_api::plan::query::decode_list_query;
use crate::tracing_routes::router::{ValidationBody, ValidationIssue, unauthenticated};

#[derive(Debug, Serialize)]
pub struct SuccessBody {
    pub status: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct HealthBody {
    pub status: HealthState,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SlackBody {
    pub status: &'static str,
    pub alert_types: Vec<String>,
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ServiceBody {
    Success(SuccessBody),
    Health(HealthBody),
    Slack(SlackBody),
}

#[derive(Debug, Serialize)]
struct TestRouteBody {
    route: String,
}

#[derive(Clone)]
pub struct HealthServicesState {
    pub mock: Arc<dyn MockRequests>,
    pub checks: Arc<dyn IntegrationChecks>,
    pub alerts: Arc<dyn AlertActions>,
    pub callbacks: Arc<dyn CallbackSource>,
    pub clock: Arc<dyn Clock>,
    pub settings: Settings,
    pub alerting: Vec<String>,
}

impl HealthServicesState {
    fn probes(&self) -> Probes<'_> {
        Probes {
            mock: self.mock.as_ref(),
            checks: self.checks.as_ref(),
            callbacks: self.callbacks.as_ref(),
            alerts: AlertSetup {
                actions: self.alerts.as_ref(),
                clock: self.clock.as_ref(),
                alerting: &self.alerting,
                test_email_address: self.settings.test_email_address.as_deref(),
            },
        }
    }
}

pub fn health_services_routes(state: HealthServicesState) -> Router {
    Router::new()
        .route("/test", get(test_endpoint))
        .route("/health/services", get(health_services_endpoint))
        .with_state(state)
}

async fn test_endpoint(request: Request) -> Json<TestRouteBody> {
    let raw = request.uri().path();
    let path = decoded_route_path(raw).unwrap_or_else(|| raw.to_owned());
    Json(TestRouteBody {
        route: request_route(&path),
    })
}

// get_request_route over the dispatched path, decoded as uvicorn decodes the request line
fn request_route(path: &str) -> String {
    let source = RequestRouteSource {
        scope: Ok(Some(ScopeFields {
            path: Some(path),
            app_root_path: None,
            root_path: None,
        })),
        url_path: path,
    };
    get_request_route(&source)
}

async fn health_services_endpoint(
    State(state): State<HealthServicesState>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw): RawQuery,
) -> Response {
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    let Some(service) = service_parameter(raw.as_deref()) else {
        return missing_service_response();
    };
    match run_service(&state, &context, &service).await {
        Ok(reply) => reply_response(reply),
        Err(error) => {
            log_service_failure(&error);
            failure_response(&error)
        }
    }
}

async fn run_service(
    state: &HealthServicesState,
    context: &AuthContext,
    text: &str,
) -> Result<Option<ServiceBody>, ServiceError> {
    let service = text
        .parse::<ServiceName>()
        .map_err(|_unparsable| ServiceError::Rejection {
            status: StatusCode::BAD_REQUEST,
            detail: unknown_service_message(text),
        })?;
    let caller = Caller::from_context(context)?;
    dispatch(service, &caller, &state.probes()).await
}

// Starlette keeps the last value of a repeated scalar, which decode_list_query's last_value map does
fn service_parameter(raw: Option<&str>) -> Option<String> {
    decode_list_query(raw.unwrap_or_default().as_bytes())
        .last_value
        .remove("service")
}

fn missing_service_response() -> Response {
    let issue = ValidationIssue {
        kind: "missing",
        loc: vec!["query".to_owned(), "service".to_owned()],
        msg: "Field required",
    };
    let body = ValidationBody {
        detail: vec![issue],
    };
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

fn reply_response(reply: Option<ServiceBody>) -> Response {
    reply.map_or_else(
        || Json(()).into_response(),
        |body| Json(body).into_response(),
    )
}

fn log_service_failure(error: &ServiceError) {
    tracing::error!(
        "litellm.proxy.proxy_server.health_services_endpoint(): Exception occured - {error}"
    );
    tracing::debug!("litellm.proxy.proxy_server.health_services_endpoint(): {error:?}");
}

fn failure_response(error: &ServiceError) -> Response {
    match error {
        ServiceError::Rejection { status, detail } => {
            let message = format!("{{'error': '{detail}'}}");
            auth_error_response(message, i64::from(status.as_u16()))
        }
        ServiceError::Caller(_)
        | ServiceError::KeyFields(_)
        | ServiceError::Test(_)
        | ServiceError::LangfuseAuthCheck { .. } => {
            auth_error_response(format!("Authentication Error, {error}"), 500)
        }
    }
}

// ProxyException stores the message as text and the code as a string, and denial_response reads both
fn auth_error_response(message: String, code: i64) -> Response {
    let exception = ProxyException::new(ProxyExceptionInit {
        message,
        error_type: "auth_error".to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(code),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    });
    denial_response(&exception)
}

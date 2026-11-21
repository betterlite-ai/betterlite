use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};

use super::conversion::AuthFailure;
use super::ports::{AuthErrorPorts, AuthLog, FailureHookCall, Level, Logger};
use crate::resolved_key::auth::{ParentOtelSpan, UserAPIKeyAuth};

// request.state is a Starlette State, and assigning to it can raise; the caller decides what that means
#[derive(Debug, thiserror::Error)]
#[error("the request state refused the assignment")]
pub struct StateRejected;

pub trait RequestState {
    fn received_at(&self) -> Option<DateTime<Utc>>;

    fn set_received_at(&mut self, at: DateTime<Utc>) -> Result<(), StateRejected>;

    fn has_parent_span(&self) -> bool;

    fn set_parent_span(&mut self, span: ParentOtelSpan);
}

pub trait ReceivedClock {
    fn now(&self) -> DateTime<Utc>;
}

// the proxy's OpenTelemetry logger, absent when none is configured
pub trait RequestTelemetry {
    fn start_request_span(
        &self,
        start: DateTime<Utc>,
        headers: &BTreeMap<String, String>,
    ) -> ParentOtelSpan;

    fn has_route_setter(&self) -> bool;

    fn set_route_attributes(&self, span: &ParentOtelSpan, url_path: &str, http_route: Option<&str>);
}

pub struct SpanRequest<'request> {
    pub headers: &'request BTreeMap<String, String>,
    pub url_path: &'request str,
    pub route_template: Option<&'request str>,
    // under V2 the HTTP instrumentation stamps the route on the server span itself
    pub otel_v2: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum BodyReadError {
    #[error("{0}")]
    Proxy(Box<ProxyException>),
    #[error("the request body could not be read")]
    Unreadable,
}

#[derive(Debug, thiserror::Error)]
#[error("the path parameters could not be added to the request body")]
pub struct PathParamsError;

#[derive(Debug, thiserror::Error)]
pub enum DeferredBodyError {
    #[error("the request body could not be read")]
    Unreadable,
    #[error(transparent)]
    PathParams(#[from] PathParamsError),
}

#[async_trait]
pub trait BodySource: Send {
    fn is_opaque_audio(&self) -> bool;

    fn cache_parsed_body(&mut self, body: JsonObject);

    async fn read_body(&mut self) -> Result<JsonObject, BodyReadError>;

    fn populate_path_params(&self, body: JsonObject) -> Result<JsonObject, PathParamsError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredBody {
    pub body: JsonObject,
    pub parse_failure: Option<ProxyException>,
}

// _ensure_litellm_received_at_on_request_state
#[must_use]
pub fn ensure_received_at<State>(state: &mut State, clock: &dyn ReceivedClock) -> DateTime<Utc>
where
    State: RequestState + ?Sized,
{
    if let Some(existing) = state.received_at() {
        return existing;
    }
    let received_at = clock.now();
    // a state that refuses the stamp still lets the request through with this moment
    match state.set_received_at(received_at) {
        Ok(()) | Err(StateRejected) => received_at,
    }
}

// _ensure_parent_otel_span_on_request_state
pub fn ensure_parent_span<State>(
    state: &mut State,
    clock: &dyn ReceivedClock,
    telemetry: Option<&dyn RequestTelemetry>,
    request: &SpanRequest<'_>,
) where
    State: RequestState + ?Sized,
{
    let start = ensure_received_at(state, clock);
    let Some(logger) = telemetry else {
        return;
    };
    if state.has_parent_span() {
        return;
    }
    let span = logger.start_request_span(start, request.headers);
    if !request.otel_v2 && logger.has_route_setter() {
        logger.set_route_attributes(&span, request.url_path, request.route_template);
    }
    state.set_parent_span(span);
}

// _read_request_body_deferring_parse_failure: only the reader's ProxyException waits for identity seeding
pub async fn read_body_deferring_parse_failure<Source>(
    source: &mut Source,
) -> Result<DeferredBody, DeferredBodyError>
where
    Source: BodySource + ?Sized,
{
    if source.is_opaque_audio() {
        source.cache_parsed_body(JsonObject::new());
        return Ok(DeferredBody {
            body: JsonObject::new(),
            parse_failure: None,
        });
    }
    let parsed = match source.read_body().await {
        Ok(parsed) => parsed,
        Err(BodyReadError::Proxy(exception)) => {
            return Ok(DeferredBody {
                body: JsonObject::new(),
                parse_failure: Some(*exception),
            });
        }
        Err(BodyReadError::Unreadable) => return Err(DeferredBodyError::Unreadable),
    };
    Ok(DeferredBody {
        body: source.populate_path_params(parsed)?,
        parse_failure: None,
    })
}

// _record_unparsable_body_failure: logging never changes the 400 the caller already holds
pub async fn record_unparsable_body_failure<Ports>(
    ports: &Ports,
    identity: &UserAPIKeyAuth,
    exception: &ProxyException,
    route: &str,
) where
    Ports: AuthErrorPorts + ?Sized,
{
    let original = AuthFailure::Proxy {
        exception: exception.clone(),
        invalid_virtual_key: false,
    };
    let empty = JsonObject::new();
    let call = FailureHookCall {
        request_data: &empty,
        original: &original,
        identity,
        error_type: ProxyErrorTypes::BadRequestError,
        route,
    };
    if let Err(error) = ports.post_call_failure_hook(call).await {
        ports.log(AuthLog {
            logger: Logger::Proxy,
            level: Level::Error,
            message: format!("Failed to log the request rejected for an unparsable body: {error}"),
            traceback: true,
            requester_ip: None,
        });
    }
}

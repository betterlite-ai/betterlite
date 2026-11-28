use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error as StdError;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::request_body::Settings as RequestBodySettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::service::management::ManagementPayload;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, FixedOffset, Utc};
use serde::Serialize;

use crate::alerting::model_alerts::key_event::VirtualKeyEvent;
use crate::failure_hook::ports::Clock;
use crate::management_effects::data::ManagementData;
use crate::management_effects::eviction::{ManagementCacheEviction, delete_cached_keys};
use crate::management_payload::alerts::{
    ManagementAlertError, ManagementAlertTransport, send_management_endpoint_alert,
};
use crate::management_payload::otel_span::{
    ManagementHttpRequest, ManagementSpanCall, ManagementSpanDeps, ManagementSpanError,
    ManagementSpanTransport, emit_management_span,
};

// Everything the route knows before the handler runs; the http_request is read only when a span is sent
pub struct ManagementCall<'call> {
    pub function_name: &'call str,
    pub request_kwargs: JsonObject,
    pub data: Option<&'call ManagementData<'call>>,
    pub user_api_key_dict: Option<&'call UserAPIKeyAuth>,
    pub parent_otel_span: Option<&'call LiveSpan>,
    pub http_request: Option<ManagementHttpRequest<'call>>,
}

// The collaborators a route passes once per call; the default auth is LiteLLM's bare UserAPIKeyAuth()
pub struct ManagementDeps<'deps, Alert, Span, Cache> {
    pub alert: Option<&'deps Alert>,
    pub span: ManagementSpanDeps<'deps, Span>,
    pub cache: &'deps Cache,
    pub default_auth: &'deps UserAPIKeyAuth,
    pub clock: Arc<dyn Clock>,
}

// The handler's failure as the route raises it: the facts the failure span logs and the error it rethrows
pub struct ManagementFailure<E> {
    pub exception: ExceptionFacts,
    pub error: E,
}

// The first failing effect ends the inner try, so every later effect is skipped
#[derive(Debug, thiserror::Error)]
enum EffectError<AlertError, SpanError, CacheError>
where
    AlertError: StdError + 'static,
    SpanError: StdError + 'static,
    CacheError: StdError + 'static,
{
    #[error(transparent)]
    Alert(ManagementAlertError<AlertError>),
    #[error(transparent)]
    Span(ManagementSpanError<SpanError>),
    #[error(transparent)]
    Cache(CacheError),
}

struct Completion<'deps, Alert, Span, Cache> {
    alert: Option<&'deps Alert>,
    span: ManagementSpanDeps<'deps, Span>,
    cache: &'deps Cache,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
}

impl<Alert, Span, Cache> Completion<'_, Alert, Span, Cache>
where
    Alert: ManagementAlertTransport,
    Span: ManagementSpanTransport,
    Cache: ManagementCacheEviction,
{
    async fn succeed<R>(self, call: ManagementCall<'_>, auth: &UserAPIKeyAuth, result: &R)
    where
        R: Serialize + Send + Sync,
    {
        if let Err(error) = self.effects(call, auth, result).await {
            tracing::debug!("Error in management endpoint wrapper: {error}");
        }
    }

    async fn effects<R>(
        self,
        call: ManagementCall<'_>,
        auth: &UserAPIKeyAuth,
        result: &R,
    ) -> Result<(), EffectError<Alert::Error, Span::Error, Cache::Error>>
    where
        R: Serialize + Send + Sync,
    {
        send_management_endpoint_alert(self.alert, call.function_name, call.request_kwargs, auth)
            .await
            .map_err(EffectError::Alert)?;
        // LiteLLM reads the parent span off the auth object, so a default auth carries no span
        if let Some(parent) = call.user_api_key_dict.and(call.parent_otel_span) {
            let response = span_response(result);
            emit_management_span(
                self.span,
                ManagementSpanCall {
                    function_name: call.function_name,
                    http_request: call.http_request,
                    parent_otel_span: Some(parent),
                    start_time: self.start_time,
                    end_time: self.end_time,
                    result: response.as_ref(),
                    exception: None,
                },
            )
            .await
            .map_err(EffectError::Span)?;
        }
        delete_cached_keys(self.cache, call.data).map_err(EffectError::Cache)?;
        Ok(())
    }

    async fn fail(self, call: ManagementCall<'_>, exception: &ExceptionFacts) {
        let Some(parent) = call.user_api_key_dict.and(call.parent_otel_span) else {
            return;
        };
        let span_call = ManagementSpanCall {
            function_name: call.function_name,
            http_request: call.http_request,
            parent_otel_span: Some(parent),
            start_time: self.start_time,
            end_time: self.end_time,
            result: None,
            exception: Some(exception),
        };
        if let Err(error) = emit_management_span(self.span, span_call).await {
            tracing::debug!(
                "Error emitting OTEL span in management endpoint wrapper failure path: {error}"
            );
        }
    }
}

// LiteLLM reads dict(result) and drops the response when that raises; a failed projection is the same
fn span_response<R>(result: &R) -> Option<Json>
where
    R: Serialize,
{
    match serde_json::to_value(result) {
        Ok(tree) => serde_json::from_value(tree).ok(),
        Err(_error) => None,
    }
}

// LiteLLM's management_endpoint_wrapper: success effects in one try, the failure span in another
pub async fn management_endpoint_wrapper<R, E, Handler, Alert, Span, Cache>(
    call: ManagementCall<'_>,
    deps: ManagementDeps<'_, Alert, Span, Cache>,
    handler: Handler,
) -> Result<R, E>
where
    R: Serialize + Send + Sync,
    E: Send,
    Handler: Future<Output = Result<R, ManagementFailure<E>>> + Send,
    Alert: ManagementAlertTransport,
    Span: ManagementSpanTransport,
    Cache: ManagementCacheEviction,
{
    let start_time = deps.clock.now().with_timezone(&Utc);
    let outcome = handler.await;
    // LiteLLM samples the end time before any effect, so effect latency stays out of the span
    let end_time = deps.clock.now().with_timezone(&Utc);
    let completion = Completion {
        alert: deps.alert,
        span: deps.span,
        cache: deps.cache,
        start_time,
        end_time,
    };
    let auth = call.user_api_key_dict.unwrap_or(deps.default_auth);
    match outcome {
        Ok(result) => {
            completion.succeed(call, auth, &result).await;
            Ok(result)
        }
        Err(failure) => {
            completion.fail(call, &failure.exception).await;
            Err(failure.error)
        }
    }
}

// LiteLLM times each call with datetime.now(), so the wrapper's clock is the wall clock
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}

// A route with no alert, parent span or cached key hands the wrapper this transport for all three effects
pub struct NoEffects;

#[async_trait]
impl ManagementAlertTransport for NoEffects {
    type Error = Infallible;

    async fn send_management_event_alert(
        &self,
        _key_event: &VirtualKeyEvent,
        _alert_type: AlertType,
        _event_name: &str,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

#[async_trait]
impl ManagementSpanTransport for NoEffects {
    type Error = Infallible;

    async fn success_hook(
        &self,
        _payload: &ManagementPayload,
        _parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Infallible> {
        Ok(())
    }

    async fn failure_hook(
        &self,
        _payload: &ManagementPayload,
        _parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

impl ManagementCacheEviction for NoEffects {
    type Error = Infallible;

    fn delete_cache(&self, _key: &str) -> Result<(), Infallible> {
        Ok(())
    }
}

// The deps a route with no alert, no parent span and no cached key hands the wrapper
#[must_use]
pub fn no_effect_deps<'deps>(
    body_settings: &'deps RequestBodySettings,
    limits: &'deps ApiLimits,
    effects: &'deps NoEffects,
    default_auth: &'deps UserAPIKeyAuth,
) -> ManagementDeps<'deps, NoEffects, NoEffects, NoEffects> {
    ManagementDeps {
        alert: None,
        span: ManagementSpanDeps {
            transport: None,
            otel_v2_enabled: false,
            body_settings,
            limits,
        },
        cache: effects,
        default_auth,
        clock: Arc::new(SystemClock),
    }
}

// LiteLLM's bare UserAPIKeyAuth() for a call that carries no key: the wrapper's default auth
pub fn default_management_auth(
    source: Option<&UserAPIKeyAuth>,
) -> Result<UserAPIKeyAuth, AuthError> {
    let Some(auth) = source else {
        return UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()));
    };
    Ok(auth.clone())
}

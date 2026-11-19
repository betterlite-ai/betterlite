use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::legacy_policy::span_time::SpanTime;
use betterlite_logging::otel::service::management::ManagementPayload;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use opentelemetry::trace::SpanContext;

use crate::alerting::model_alerts::key_event::VirtualKeyEvent;
use crate::management_payload::alerts::ManagementAlertTransport;
use crate::management_payload::otel_span::ManagementSpanTransport;

use super::journal::{Effect, Journal};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FakeTransportError {
    #[error("the fake transport failed")]
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct SentManagementAlert {
    pub(crate) created_by_user_id: String,
    pub(crate) created_by_user_role: String,
    pub(crate) created_by_key_alias: Option<String>,
    pub(crate) alert_type: AlertType,
    pub(crate) request_kwargs: JsonObject,
}

#[derive(Clone)]
pub(crate) struct RecordingAlertTransport {
    pub(crate) events: Journal<Effect>,
    pub(crate) sent: Journal<SentManagementAlert>,
    pub(crate) fails: bool,
}

#[async_trait]
impl ManagementAlertTransport for RecordingAlertTransport {
    type Error = FakeTransportError;

    async fn send_management_event_alert(
        &self,
        key_event: &VirtualKeyEvent,
        alert_type: AlertType,
        _event_name: &str,
    ) -> Result<(), Self::Error> {
        if self.fails {
            return Err(FakeTransportError::Failed);
        }
        self.events.record(Effect::Alert);
        self.sent.record(SentManagementAlert {
            created_by_user_id: key_event.created_by_user_id.clone(),
            created_by_user_role: key_event.created_by_user_role.clone(),
            created_by_key_alias: key_event.created_by_key_alias.clone(),
            alert_type,
            request_kwargs: key_event.request_kwargs.clone(),
        });
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpanHook {
    Success,
    Failure,
}

#[derive(Clone)]
pub(crate) struct SentManagementSpan {
    pub(crate) hook: SpanHook,
    pub(crate) route: String,
    pub(crate) response: Option<Json>,
    pub(crate) exception: Option<ExceptionFacts>,
    pub(crate) start_time: SpanTime,
    pub(crate) end_time: SpanTime,
    pub(crate) parent: Option<SpanContext>,
}

#[derive(Clone)]
pub(crate) struct RecordingSpanTransport {
    pub(crate) events: Journal<Effect>,
    pub(crate) sent: Journal<SentManagementSpan>,
    pub(crate) fails: bool,
}

impl RecordingSpanTransport {
    fn record(
        &self,
        hook: SpanHook,
        payload: &ManagementPayload,
        parent: Option<&LiveSpan>,
    ) -> Result<(), FakeTransportError> {
        if self.fails {
            return Err(FakeTransportError::Failed);
        }
        match hook {
            SpanHook::Success => self.events.record(Effect::SuccessSpan),
            SpanHook::Failure => self.events.record(Effect::FailureSpan),
        }
        self.sent.record(SentManagementSpan {
            hook,
            route: payload.route.clone(),
            response: response_json(payload),
            exception: payload.exception.clone(),
            start_time: payload.start_time,
            end_time: payload.end_time,
            parent: parent.map(|span| span.context.clone()),
        });
        Ok(())
    }
}

#[async_trait]
impl ManagementSpanTransport for RecordingSpanTransport {
    type Error = FakeTransportError;

    async fn success_hook(
        &self,
        payload: &ManagementPayload,
        parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Self::Error> {
        self.record(SpanHook::Success, payload, parent_otel_span)
    }

    async fn failure_hook(
        &self,
        payload: &ManagementPayload,
        parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Self::Error> {
        self.record(SpanHook::Failure, payload, parent_otel_span)
    }
}

// The payload carries the legacy span value type; the fixture compares betterlite's own JSON tree
fn response_json(payload: &ManagementPayload) -> Option<Json> {
    let fields = payload.response.as_ref()?;
    let tree = serde_json::to_value(fields).expect("the span response serializes");
    Some(serde_json::from_value(tree).expect("the span response is JSON"))
}

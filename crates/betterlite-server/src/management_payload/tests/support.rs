#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_route_patterns::canonical::RequestRouteSource;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::request_body::Settings as RequestBodySettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::RequestBodyInput;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::legacy_policy::span_time::SpanTime;
use betterlite_logging::otel::service::management::ManagementPayload;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use opentelemetry::trace::SpanContext;
use serde::Serialize;

use crate::alerting::model_alerts::key_event::VirtualKeyEvent;
use crate::management_payload::alerts::ManagementAlertTransport;
use crate::management_payload::otel_span::{
    ManagementHttpRequest, ManagementSpanCall, ManagementSpanDeps, ManagementSpanError,
    ManagementSpanTransport, emit_management_endpoint_otel_span,
};

#[derive(Debug, thiserror::Error)]
#[error("the fake transport failed")]
pub(crate) struct FakeTransportError;

#[derive(Clone)]
pub(crate) struct Journal<T>(Arc<Mutex<Vec<T>>>);

impl<T> Default for Journal<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl<T: Clone> Journal<T> {
    pub(crate) fn record(&self, entry: T) {
        self.0.lock().expect("journal lock").push(entry);
    }

    pub(crate) fn entries(&self) -> Vec<T> {
        self.0.lock().expect("journal lock").clone()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SentManagementAlert {
    pub(crate) created_by_user_id: String,
    pub(crate) created_by_user_role: String,
    pub(crate) created_by_key_alias: Option<String>,
    pub(crate) request_kwargs: JsonObject,
    pub(crate) alert_type: AlertType,
    pub(crate) event_name: String,
}

#[derive(Clone, Default)]
pub(crate) struct RecordingAlertTransport {
    pub(crate) alerts: Journal<SentManagementAlert>,
    pub(crate) fails: bool,
}

impl RecordingAlertTransport {
    pub(crate) fn failing() -> Self {
        Self {
            alerts: Journal::default(),
            fails: true,
        }
    }
}

#[async_trait]
impl ManagementAlertTransport for RecordingAlertTransport {
    type Error = FakeTransportError;

    async fn send_management_event_alert(
        &self,
        key_event: &VirtualKeyEvent,
        alert_type: AlertType,
        event_name: &str,
    ) -> Result<(), Self::Error> {
        if self.fails {
            return Err(FakeTransportError);
        }
        self.alerts.record(SentManagementAlert {
            created_by_user_id: key_event.created_by_user_id.clone(),
            created_by_user_role: key_event.created_by_user_role.clone(),
            created_by_key_alias: key_event.created_by_key_alias.clone(),
            request_kwargs: key_event.request_kwargs.clone(),
            alert_type,
            event_name: event_name.to_owned(),
        });
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpanHook {
    Success,
    Failure,
}

#[derive(Clone, Debug)]
pub(crate) struct SentManagementSpan {
    pub(crate) hook: SpanHook,
    pub(crate) route: String,
    pub(crate) request_data: Json,
    pub(crate) response: Option<Json>,
    pub(crate) exception: Option<ExceptionFacts>,
    pub(crate) start_time: SpanTime,
    pub(crate) end_time: SpanTime,
    pub(crate) parent: Option<SpanContext>,
}

#[derive(Clone, Default)]
pub(crate) struct RecordingSpanTransport {
    pub(crate) spans: Journal<SentManagementSpan>,
    pub(crate) fails: bool,
}

impl RecordingSpanTransport {
    pub(crate) fn failing() -> Self {
        Self {
            spans: Journal::default(),
            fails: true,
        }
    }

    fn record(
        &self,
        hook: SpanHook,
        payload: &ManagementPayload,
        parent: Option<&LiveSpan>,
    ) -> Result<(), FakeTransportError> {
        if self.fails {
            return Err(FakeTransportError);
        }
        self.spans.record(SentManagementSpan {
            hook,
            route: payload.route.clone(),
            request_data: json_value(&payload.request_data),
            response: payload.response.as_ref().map(json_value),
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

    async fn management_endpoint_success_hook(
        &self,
        payload: &ManagementPayload,
        parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Self::Error> {
        self.record(SpanHook::Success, payload, parent_otel_span)
    }

    async fn management_endpoint_failure_hook(
        &self,
        payload: &ManagementPayload,
        parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Self::Error> {
        self.record(SpanHook::Failure, payload, parent_otel_span)
    }
}

#[derive(Default)]
pub(crate) struct SpanHarness {
    pub(crate) transport: RecordingSpanTransport,
    pub(crate) settings: RequestBodySettings,
    pub(crate) limits: ApiLimits,
}

impl SpanHarness {
    pub(crate) fn failing() -> Self {
        Self {
            transport: RecordingSpanTransport::failing(),
            ..Self::default()
        }
    }

    pub(crate) async fn emit(
        &self,
        call: ManagementSpanCall<'_>,
    ) -> Result<(), ManagementSpanError<FakeTransportError>> {
        self.emit_with(call, false).await
    }

    pub(crate) async fn emit_v2(
        &self,
        call: ManagementSpanCall<'_>,
    ) -> Result<(), ManagementSpanError<FakeTransportError>> {
        self.emit_with(call, true).await
    }

    async fn emit_with(
        &self,
        call: ManagementSpanCall<'_>,
        otel_v2_enabled: bool,
    ) -> Result<(), ManagementSpanError<FakeTransportError>> {
        emit_management_endpoint_otel_span(
            ManagementSpanDeps {
                transport: Some(&self.transport),
                otel_v2_enabled,
                body_settings: &self.settings,
                limits: &self.limits,
            },
            call,
        )
        .await
    }
}

pub(crate) async fn emit_without_transport(
    call: ManagementSpanCall<'_>,
) -> Result<(), ManagementSpanError<FakeTransportError>> {
    let settings = RequestBodySettings::default();
    let limits = ApiLimits::default();
    emit_management_endpoint_otel_span::<RecordingSpanTransport>(
        ManagementSpanDeps {
            transport: None,
            otel_v2_enabled: false,
            body_settings: &settings,
            limits: &limits,
        },
        call,
    )
    .await
}

pub(crate) fn span_call<'call>(
    function_name: &'call str,
    result: Option<&'call Json>,
    exception: Option<&'call ExceptionFacts>,
) -> ManagementSpanCall<'call> {
    ManagementSpanCall {
        function_name,
        http_request: None,
        parent_otel_span: None,
        start_time: start_time(),
        end_time: end_time(),
        result,
        exception,
    }
}

pub(crate) fn json_request<'path>(path: &'path str, body: &str) -> ManagementHttpRequest<'path> {
    ManagementHttpRequest {
        route: RequestRouteSource {
            scope: Ok(None),
            url_path: path,
        },
        body: RequestBodyInput {
            method: "POST".to_owned(),
            route_path: path.to_owned(),
            headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
            body: body.as_bytes().to_vec(),
            ..RequestBodyInput::default()
        },
    }
}

pub(crate) fn start_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_704_067_200, 0).expect("a valid start timestamp")
}

pub(crate) fn end_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_704_067_202, 0).expect("a valid end timestamp")
}

pub(crate) fn exception() -> ExceptionFacts {
    ExceptionFacts {
        class_name: "ProxyException".to_owned(),
        message: Some("boom".to_owned()),
        ..ExceptionFacts::default()
    }
}

pub(crate) fn key_fixture(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields parse");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

pub(crate) fn json_value(value: impl Serialize) -> Json {
    let tree = serde_json::to_value(value).expect("the value serializes");
    serde_json::from_value(tree).expect("the value is JSON")
}

pub(crate) fn json_object(value: impl Serialize) -> JsonObject {
    let Json::Object(fields) = json_value(value) else {
        panic!("the fixture is a JSON object");
    };
    fields
}

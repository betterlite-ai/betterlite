use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_auth::auth_route_patterns::canonical::{RequestRouteSource, get_request_route};
use betterlite_config::sections::request_body::Settings as RequestBodySettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{RequestBodyError, RequestBodyInput};
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::legacy_policy::span_time::SpanTime;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::service::management::ManagementPayload;
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};

use crate::api::request_body::read_request_body;
use crate::management_payload::redaction::redact_env_var_values;

// LiteLLM's _CREDENTIAL_FIELDS: the top-level response keys removed before telemetry
const CREDENTIAL_FIELDS: [&str; 9] = [
    "key",
    "token",
    "api_key",
    "secret",
    "password",
    "access_token",
    "refresh_token",
    "private_key",
    "service_account_key",
];

// The http_request a management handler receives, read through B741's route and B1100's body parser
pub struct ManagementHttpRequest<'source> {
    pub route: RequestRouteSource<'source>,
    pub body: RequestBodyInput,
}

// The facts of one management call; the transport and settings arrive through ManagementSpanDeps
pub struct ManagementSpanCall<'call> {
    pub function_name: &'call str,
    pub http_request: Option<ManagementHttpRequest<'call>>,
    pub parent_otel_span: Option<&'call LiveSpan>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub result: Option<&'call Json>,
    pub exception: Option<&'call ExceptionFacts>,
}

pub struct ManagementSpanDeps<'deps, Transport> {
    pub transport: Option<&'deps Transport>,
    pub otel_v2_enabled: bool,
    pub body_settings: &'deps RequestBodySettings,
    pub limits: &'deps ApiLimits,
}

#[derive(Debug, thiserror::Error)]
pub enum ManagementSpanError<E>
where
    E: StdError + 'static,
{
    #[error("the management request body could not be read")]
    Body(#[source] RequestBodyError),
    #[error("the management request body could not be projected")]
    RequestData(#[source] serde_json::Error),
    #[error("the management span could not be sent")]
    Span(#[source] E),
}

// B110 adapts B3858's management success and failure hooks to this port
#[async_trait]
pub trait ManagementSpanTransport: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn success_hook(
        &self,
        payload: &ManagementPayload,
        parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Self::Error>;

    async fn failure_hook(
        &self,
        payload: &ManagementPayload,
        parent_otel_span: Option<&LiveSpan>,
    ) -> Result<(), Self::Error>;
}

// LiteLLM's _emit_management_endpoint_otel_span: a missing logger and the V2 span mode both skip every step
pub async fn emit_management_span<Transport>(
    deps: ManagementSpanDeps<'_, Transport>,
    call: ManagementSpanCall<'_>,
) -> Result<(), ManagementSpanError<Transport::Error>>
where
    Transport: ManagementSpanTransport,
{
    let Some(transport) = deps.transport else {
        return Ok(());
    };
    if deps.otel_v2_enabled {
        return Ok(());
    }
    let payload = span_payload::<Transport::Error>(&call, deps.body_settings, deps.limits)?;
    send_hook(
        transport,
        &payload,
        call.exception.is_some(),
        call.parent_otel_span,
    )
    .await
}

// the success and failure hooks are distinct transports, so the exception picks exactly one
async fn send_hook<Transport>(
    transport: &Transport,
    payload: &ManagementPayload,
    failed: bool,
    parent: Option<&LiveSpan>,
) -> Result<(), ManagementSpanError<Transport::Error>>
where
    Transport: ManagementSpanTransport,
{
    let sent = if failed {
        transport.failure_hook(payload, parent).await
    } else {
        transport.success_hook(payload, parent).await
    };
    sent.map_err(ManagementSpanError::Span)
}

fn span_payload<E>(
    call: &ManagementSpanCall<'_>,
    body_settings: &RequestBodySettings,
    limits: &ApiLimits,
) -> Result<ManagementPayload, ManagementSpanError<E>>
where
    E: StdError + 'static,
{
    let (route, request_data) = request_metadata::<E>(call, body_settings, limits)?;
    let response = if call.exception.is_none() {
        call.result.and_then(sanitized_response)
    } else {
        None
    };
    Ok(ManagementPayload {
        route,
        request_data,
        response: response.map(metadata_map),
        exception: call.exception.cloned(),
        start_time: SpanTime::DateTime(call.start_time),
        end_time: SpanTime::DateTime(call.end_time),
    })
}

// an endpoint without http_request falls back to the handler's name and an empty body
fn request_metadata<E>(
    call: &ManagementSpanCall<'_>,
    body_settings: &RequestBodySettings,
    limits: &ApiLimits,
) -> Result<(String, MetadataMap), ManagementSpanError<E>>
where
    E: StdError + 'static,
{
    let Some(request) = call.http_request.as_ref() else {
        return Ok((call.function_name.to_owned(), MetadataMap::new()));
    };
    let route = get_request_route(&request.route);
    let body = read_request_body(&request.body, body_settings, limits)
        .map_err(ManagementSpanError::Body)?;
    let tree = serde_json::to_value(body.body).map_err(ManagementSpanError::RequestData)?;
    let fields =
        serde_json::from_value::<MetadataMap>(tree).map_err(ManagementSpanError::RequestData)?;
    Ok((route, fields))
}

// LiteLLM's `try: dict(result) ... except Exception: response = None`, which a non-mapping result takes
fn sanitized_response(result: &Json) -> Option<JsonObject> {
    let Json::Object(mapping) = result else {
        return None;
    };
    let mut sanitized = mapping.clone();
    sanitized.retain(|key, _value| !CREDENTIAL_FIELDS.contains(&key.as_str()));
    redact_env_var_values(&mut sanitized);
    Some(sanitized)
}

// The response arrives as betterlite's JSON tree; this walk projects it onto the span's value type
fn metadata_map(fields: JsonObject) -> MetadataMap {
    fields
        .into_iter()
        .map(|(key, value)| (key, metadata_value(value)))
        .collect()
}

fn metadata_value(value: Json) -> OtelModelValue {
    match value {
        Json::Null => OtelModelValue::Null(()),
        Json::Bool(flag) => OtelModelValue::Bool(flag),
        Json::Number(number) => OtelModelValue::Number(number),
        Json::Text(text) => OtelModelValue::String(text),
        Json::List(items) => OtelModelValue::Array(items.into_iter().map(metadata_value).collect()),
        Json::Object(fields) => OtelModelValue::Object(metadata_map(fields)),
    }
}

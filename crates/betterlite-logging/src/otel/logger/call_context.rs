use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use chrono::{Offset as _, Utc};
use opentelemetry::Context;
use rust_decimal::Decimal;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::otel::config::Settings;
use crate::otel::model::baggage::promoted_baggage;
use crate::otel::model::metadata::llm_call_event::LLMCallEvent;
use crate::otel::model::metadata::wire::MetadataError;
use crate::otel::model::metadata::wire::callback::{
    Callback, OptionalParams, Params, ProxyRequest,
};
use crate::otel::model::metadata::wire::timestamp::Timestamp;
use crate::otel::model::payloads::RequestIdentity;
use crate::otel::model::utils::time::{FixedLocalTimeResolver, to_ns};
use crate::otel::plumbing::context::baggage_context::set_request_baggage;

// LiteLLM's litellm_no_upstream_llm_call kwargs flag, which the proxy sets on a synthetic gate log
const NO_UPSTREAM_LLM_CALL_KEY: &str = "litellm_no_upstream_llm_call";

// the closed request as the OTel callbacks read it
pub struct CallContext {
    pub event: LLMCallEvent,
    pub dynamic_params: Arc<StandardCallbackDynamicParams>,
    pub stream: bool,
    pub api_call_start_time: Decimal,
    pub start_time: Decimal,
    pub end_time: Decimal,
    pub completion_start_time: Decimal,
}

impl CallContext {
    // the request's own start stands in for a missing carrier, so a payload still reads as upstream
    pub fn from_event(
        event: &LoggerEvent,
        api_start: Option<SystemTime>,
    ) -> Result<Self, MetadataError> {
        let start_time = event.payload.start_time;
        let api_call_start_time = api_start
            .and_then(epoch_seconds)
            .or(Some(start_time))
            .unwrap_or_default()
            .max(Decimal::ZERO);
        let callback = wire_callback(event, api_call_start_time)?;
        Ok(Self {
            event: LLMCallEvent::from_dict(&callback)?,
            dynamic_params: Arc::clone(&event.dynamic_params),
            stream: event.stream,
            api_call_start_time,
            start_time,
            end_time: event.payload.end_time,
            completion_start_time: event.payload.completion_start_time,
        })
    }
}

// the kwargs dict LiteLLM hands its success and failure callbacks, rebuilt from the typed event
fn wire_callback(
    event: &LoggerEvent,
    api_call_start_time: Decimal,
) -> Result<Callback, MetadataError> {
    let payload = &event.payload;
    Ok(Callback {
        standard_logging_object: Some(serde_json::from_value(serde_json::to_value(&**payload)?)?),
        litellm_call_id: payload.litellm_call_id.clone(),
        model: Some(payload.model.clone()),
        call_type: Some(payload.call_type.clone()),
        dynamic_params: Some(serde_json::from_value(serde_json::to_value(
            &*event.dynamic_params,
        )?)?),
        litellm_no_upstream_llm_call: event
            .litellm_metadata
            .get(NO_UPSTREAM_LLM_CALL_KEY)
            .is_some_and(|value| matches!(value, Json::Bool(true))),
        api_call_start_time: (api_call_start_time > Decimal::ZERO)
            .then_some(Timestamp(api_call_start_time)),
        completion_start_time: (payload.completion_start_time > Decimal::ZERO)
            .then_some(Timestamp(payload.completion_start_time)),
        optional_params: OptionalParams {
            stream: event.stream,
        },
        litellm_params: Params {
            metadata: serde_json::from_value(serde_json::to_value(&payload.metadata)?)?,
            litellm_metadata: serde_json::from_value(serde_json::to_value(
                &*event.litellm_metadata,
            )?)?,
            litellm_session_id: Some(payload.session_id.clone()),
            proxy_server_request: ProxyRequest { headers: None },
        },
    })
}

// litellm.integrations.otel.logger.OpenTelemetryV2._seed_identity_baggage reads the parsed payload's identity, never the carrier
#[must_use]
pub fn seed_identity_baggage(
    config: &Settings,
    identity: &RequestIdentity,
    model: Option<&str>,
    context: Context,
) -> Context {
    let bag = promoted_baggage(
        identity,
        model,
        &config.baggage_promoted_keys,
        &config.baggage_metadata_keys,
        &config.baggage_team_metadata_keys,
    );
    if bag.is_empty() {
        context
    } else {
        set_request_baggage(&bag, Some(&context))
    }
}

// the callback's start and end instants, as LiteLLM's to_ns reads the datetimes it is handed
#[derive(Clone, Copy, Debug, Default)]
pub struct SpanTimes {
    pub start: Option<SystemTime>,
    pub end: Option<SystemTime>,
}

impl SpanTimes {
    #[must_use]
    pub fn from_payload(payload: &StandardLoggingPayload) -> Self {
        Self {
            start: system_time(payload.start_time),
            end: system_time(payload.end_time),
        }
    }
}

// the payload stamps are epoch seconds, so to_ns never consults the resolver it takes alongside them
#[must_use]
pub fn system_time(seconds: Decimal) -> Option<SystemTime> {
    let stamp = to_ns(
        Some(&seconds),
        None,
        &FixedLocalTimeResolver { offset: Utc.fix() },
    )?;
    let nanos = u64::try_from(stamp).ok()?;
    UNIX_EPOCH.checked_add(Duration::from_nanos(nanos))
}

// epoch seconds, the form the metric recorder's latency stamps read
#[must_use]
pub fn epoch_seconds(time: SystemTime) -> Option<Decimal> {
    let elapsed = time.duration_since(UNIX_EPOCH).ok()?;
    let fraction = Decimal::new(i64::from(elapsed.subsec_nanos()), 9);
    Decimal::from(elapsed.as_secs()).checked_add(fraction)
}

use chrono::SecondsFormat;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::finops::metering::clock::UtcClock;
use crate::finops::metering::event::MeteringEvent;
use crate::finops::metering::openmeter::settings::OpenMeterSettings;

pub const SPEC_VERSION: &str = "1.0";
pub const SOURCE: &str = "litellm-proxy";

#[derive(Debug, Serialize)]
pub struct CloudEvent {
    pub specversion: &'static str,
    #[serde(rename = "type")]
    pub event_type: String,
    pub id: Option<String>,
    pub time: String,
    pub subject: String,
    pub source: &'static str,
    pub data: CloudEventData,
}

#[derive(Debug, Serialize)]
pub struct CloudEventData {
    pub model: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenMeterPayloadError {
    #[error("OpenMeter needs a request user or user_api_key_user_id for the event subject")]
    MissingSubject,
}

pub fn build<Time>(
    settings: &OpenMeterSettings,
    clock: &Time,
    event: &MeteringEvent<'_>,
) -> Result<CloudEvent, OpenMeterPayloadError>
where
    Time: UtcClock,
{
    let trusted_user = if settings.trust_request_user {
        event.request_user.or(event.user_api_key_user_id)
    } else {
        event.user_api_key_user_id
    };
    let Some(subject) = trusted_user else {
        return Err(OpenMeterPayloadError::MissingSubject);
    };
    Ok(CloudEvent {
        specversion: SPEC_VERSION,
        event_type: settings.event_type.clone(),
        id: event
            .response_id
            .or(event.litellm_call_id)
            .map(str::to_owned),
        // LiteLLM keeps Python isoformat's +00:00 offset instead of a Z suffix
        time: clock.now().to_rfc3339_opts(SecondsFormat::Micros, false),
        subject: subject.to_owned(),
        source: SOURCE,
        data: CloudEventData {
            model: event.model.map(str::to_owned),
            cost: event.response_cost,
            prompt_tokens: event.usage.map(|usage| usage.prompt_tokens),
            completion_tokens: event.usage.map(|usage| usage.completion_tokens),
            total_tokens: event.usage.map(|usage| usage.total_tokens),
        },
    })
}

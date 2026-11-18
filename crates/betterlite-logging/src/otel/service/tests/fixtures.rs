use std::collections::BTreeMap;

use betterlite_error::normalize::Normalizer;

use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::service::ServiceLoggerPayload;
use crate::otel::model::payloads::service::ServiceType;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::service::hooks::ServiceSpanCall;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;

use super::support::{END, START, carry_in};

pub(super) fn payload(
    service: ServiceType,
    call_type: &str,
    caller: Option<&str>,
    error: Option<&str>,
) -> ServiceLoggerPayload {
    ServiceLoggerPayload {
        service,
        call_type: call_type.to_owned(),
        caller: caller.map(str::to_owned),
        error: error.map(str::to_owned),
    }
}

pub(super) fn service_call<'call>(
    payload: &'call ServiceLoggerPayload,
    parent: Option<&'call LiveSpan>,
    event_metadata: Option<&'call MetadataMap>,
) -> ServiceSpanCall<'call> {
    ServiceSpanCall {
        payload,
        parent,
        start_time: SpanTime::Seconds(START),
        end_time: SpanTime::Seconds(END),
        event_metadata,
    }
}

pub(super) fn database_env() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "DATABASE_URL".to_owned(),
        "postgres://user:pass@db.example.com:6543/appdb".to_owned(),
    )])
}

pub(super) fn guardrail_entry() -> OtelModelValue {
    serde_json::from_str(
        r#"{
        "guardrail_name": "pii",
        "guardrail_mode": "pre_call",
        "guardrail_status": "success",
        "start_time": 1704067200.5,
        "end_time": 1704067200.75
    }"#,
    )
    .expect("guardrail entry")
}

fn team_metadata() -> MetadataMap {
    MetadataMap::from([
        (
            "user_api_key_team_id".to_owned(),
            OtelModelValue::String("team-a".to_owned()),
        ),
        (
            "user_api_key_team_alias".to_owned(),
            OtelModelValue::String("team-alias".to_owned()),
        ),
    ])
}

// the proxy-internal metadata bucket request_data and kwargs both carry
pub(super) fn request_data(call_id: &str, entries: Vec<OtelModelValue>) -> MetadataMap {
    let mut metadata = team_metadata();
    metadata.insert(
        "litellm_call_id".to_owned(),
        OtelModelValue::String(call_id.to_owned()),
    );
    metadata.insert(
        "standard_logging_guardrail_information".to_owned(),
        OtelModelValue::Array(entries),
    );
    MetadataMap::from([("metadata".to_owned(), OtelModelValue::Object(metadata))])
}

// Logging.model_call_details as the success hook reads it
pub(super) fn details(
    call_id: &str,
    entries: Option<Vec<OtelModelValue>>,
    parent: Option<&LiveSpan>,
) -> MetadataMap {
    let mut metadata = team_metadata();
    metadata.insert(
        "litellm_call_id".to_owned(),
        OtelModelValue::String(call_id.to_owned()),
    );
    let mut logged = MetadataMap::from([(
        "metadata".to_owned(),
        OtelModelValue::Object(team_metadata()),
    )]);
    if let Some(information) = entries {
        logged.insert(
            "guardrail_information".to_owned(),
            OtelModelValue::Array(information),
        );
    }
    let mut details = MetadataMap::from([
        (
            "litellm_params".to_owned(),
            OtelModelValue::Object(MetadataMap::from([(
                "metadata".to_owned(),
                OtelModelValue::Object(metadata),
            )])),
        ),
        (
            "standard_logging_object".to_owned(),
            OtelModelValue::Object(logged),
        ),
    ]);
    if let Some(span) = parent {
        carry_in(&mut details, span);
    }
    details
}

pub(super) fn exception(json: &str) -> ExceptionFacts {
    serde_json::from_str(json).expect("exception fixture")
}

pub(super) fn number(value: f64) -> OtelModelValue {
    OtelModelValue::Number(serde_json::Number::from_f64(value).expect("finite number"))
}

pub(super) fn error_config() -> ErrorInformationConfig {
    ErrorInformationConfig {
        normalizer: Normalizer::new().expect("normalizer"),
        redactor: None,
        log_client_error_tracebacks: false,
    }
}

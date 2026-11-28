#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::api::realtime_http_calls::error::{RealtimeHttpCallsError, RecordError};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionError;
use betterlite_provider::api::realtime_http_transport::error::{
    ProviderError, RealtimeHttpError, StatusFailure, TimeoutFailure,
};
use indexmap::IndexMap;
use serde_json::Number;

use super::{ConvertError, Payload, calls, client_secret, http_failure};

// One payload the way main.py writes it: the arguments object and nothing else
fn arguments(payload: &str) -> super::Arguments {
    serde_json::from_str::<Payload>(payload).unwrap().arguments
}

#[test]
fn request_client_secret_keeps_its_arguments() {
    let payload = r#"{"arguments":{"model":"gpt-4o-realtime-preview","session":{"type":"realtime","model":"m"},"expires_after":{"anchor":"created_at","seconds":60},"timeout":30.0,"litellm_logging_obj":{"call_type":"realtime"}}}"#;
    let request = client_secret(arguments(payload));
    assert_eq!(
        request.common.model.as_deref(),
        Some("gpt-4o-realtime-preview")
    );
    assert_eq!(request.common.timeout, Some(30.0_f64));
    assert_eq!(
        request.session,
        Some(Json::Object(IndexMap::from([
            (String::from("type"), Json::Text(String::from("realtime"))),
            (String::from("model"), Json::Text(String::from("m"))),
        ])))
    );
    assert_eq!(
        request.expires_after,
        Some(Json::Object(IndexMap::from([
            (
                String::from("anchor"),
                Json::Text(String::from("created_at"))
            ),
            (String::from("seconds"), Json::Number(Number::from(60_i64))),
        ])))
    );
    assert!(request.common.kwargs.contains_key("litellm_logging_obj"));
}

#[test]
fn request_transcription_keeps_the_session_body() {
    let payload = r#"{"arguments":{"model":"gpt-realtime-whisper","transcription_session":{"input_audio_transcription":{"model":"gpt-4o-transcribe"}}}}"#;
    let request = super::transcription(arguments(payload));
    assert_eq!(
        request.common.model.as_deref(),
        Some("gpt-realtime-whisper")
    );
    assert_eq!(
        request.transcription_session,
        Some(Json::Object(IndexMap::from([(
            String::from("input_audio_transcription"),
            Json::Object(IndexMap::from([(
                String::from("model"),
                Json::Text(String::from("gpt-4o-transcribe")),
            )])),
        )])))
    );
}

#[test]
fn request_calls_requires_the_ephemeral_key() {
    let payload = r#"{"arguments":{"model":"gpt-4o-realtime-preview"}}"#;
    let error = calls(arguments(payload), vec![b"sdp".to_vec()])
        .err()
        .unwrap();
    assert!(matches!(
        error,
        ConvertError::Missing("openai_ephemeral_key")
    ));
}

#[test]
fn request_calls_takes_the_sdp_from_the_last_blob() {
    let payload = r#"{"arguments":{"openai_ephemeral_key":"ek_1","model":"gpt-4o-realtime-preview","session":{"type":"realtime"}}}"#;
    let request = calls(arguments(payload), vec![b"offer".to_vec()]).unwrap();
    assert_eq!(request.ephemeral_key.as_str(), "ek_1");
    assert_eq!(request.sdp, b"offer".to_vec());
    assert_eq!(
        request.session,
        Some(Json::Object(IndexMap::from([(
            String::from("type"),
            Json::Text(String::from("realtime")),
        )])))
    );
}

#[test]
fn error_provider_keeps_class_status_and_headers() {
    let failure = http_failure(&RealtimeHttpCallsError::Dispatch(
        RealtimeHttpError::Provider(ProviderError {
            status_code: 401,
            message: String::from("no key"),
            headers: BTreeMap::from([(
                String::from("x-request-id"),
                Json::Text(String::from("r1")),
            )]),
            status_code_is_synthesized: false,
        }),
    ));
    assert_eq!(failure.kind.as_str(), "BaseLLMException");
    assert_eq!(failure.message.as_str(), "no key");
    assert_eq!(failure.status_code, Some(401));
    assert_eq!(
        failure.headers,
        Some(BTreeMap::from([(
            String::from("x-request-id"),
            String::from("r1")
        )]))
    );
}

#[test]
fn error_status_is_the_masked_status_error() {
    let failure = http_failure(&RealtimeHttpCallsError::Dispatch(
        RealtimeHttpError::Status(StatusFailure {
            status_code: 500,
            message: String::from("boom"),
            headers: BTreeMap::new(),
        }),
    ));
    assert_eq!(failure.kind.as_str(), "MaskedHTTPStatusError");
    assert_eq!(failure.status_code, Some(500));
}

#[test]
fn error_timeout_is_litellm_timeout() {
    let failure = http_failure(&RealtimeHttpCallsError::Dispatch(
        RealtimeHttpError::Timeout(TimeoutFailure {
            message: String::from("litellm.Timeout: Connection timed out. Timeout passed=1"),
        }),
    ));
    assert_eq!(failure.kind.as_str(), "Timeout");
    assert_eq!(failure.status_code, None);
}

#[test]
fn error_resolution_is_a_bad_request() {
    let failure = http_failure(&RealtimeHttpCallsError::Resolution(
        ResolutionError::MissingProvider {
            model: String::from("gpt-4o-realtime-preview"),
        },
    ));
    assert_eq!(failure.kind.as_str(), "BadRequestError");
    assert_eq!(failure.status_code, Some(400));
}

#[test]
fn error_transport_is_a_connection_error() {
    let failure = http_failure(&RealtimeHttpCallsError::Dispatch(
        RealtimeHttpError::Transport(TransportError::BodyTooLarge { limit: 8 }),
    ));
    assert_eq!(failure.kind.as_str(), "APIConnectionError");
    assert_eq!(failure.status_code, None);
}

#[test]
fn error_record_is_a_value_error() {
    let source = serde_json::from_str::<i32>("not a number").unwrap_err();
    let failure = http_failure(&RealtimeHttpCallsError::Record(RecordError::Invalid {
        field: "session",
        source,
    }));
    assert_eq!(failure.kind.as_str(), "ValueError");
}

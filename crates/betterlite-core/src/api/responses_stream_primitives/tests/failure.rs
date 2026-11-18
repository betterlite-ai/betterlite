use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rstest::rstest;

use crate::api::responses_stream_primitives::error_fields::{
    StreamErrorObject, error_event_fields,
};
use crate::api::responses_stream_primitives::failure::{
    map_stream_error_to_exception, mid_stream_fallback_eligible, provider_error_message,
};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn mapped(pairs: &[(&str, Json)]) -> ErrorClass {
    map_stream_error_to_exception(
        StreamErrorObject::Mapping(&object(pairs)),
        "gpt-4o",
        "openai",
        &redactor(),
    )
}

fn content_policy() -> ErrorClass {
    mapped(&[
        ("message", text("blocked")),
        ("type", text("invalid_request_error")),
        ("code", text("content_policy_violation")),
    ])
}

#[test]
fn provider_error_message_uses_pythons_dict_repr() {
    let fields = error_event_fields(StreamErrorObject::Mapping(&object(&[
        ("message", text("slow")),
        ("code", text("rate_limit_exceeded")),
    ])));
    assert_eq!(
        provider_error_message(&fields, 429),
        "Error code: 429 - {'error': {'message': 'slow', 'type': None, 'code': 'rate_limit_exceeded'}}"
    );
}

#[test]
fn provider_error_message_quotes_a_message_with_an_apostrophe() {
    let fields = error_event_fields(StreamErrorObject::Mapping(&object(&[(
        "message",
        text("it's slow"),
    )])));
    assert_eq!(
        provider_error_message(&fields, 500),
        "Error code: 500 - {'error': {'message': \"it's slow\", 'type': None, 'code': None}}"
    );
}

#[test]
fn map_stream_error_to_exception_maps_a_rate_limit() {
    let class = mapped(&[
        ("message", text("slow")),
        ("code", text("rate_limit_exceeded")),
    ]);
    assert!(matches!(class, ErrorClass::RateLimit(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 429);
    assert_eq!(metadata.code, "429");
    assert_eq!(
        metadata.error_type,
        WireField::Text("throttling_error".to_owned())
    );
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code.as_deref(), Some("429"));
    assert_eq!(metadata.model.as_deref(), Some("gpt-4o"));
    assert_eq!(metadata.provider.as_deref(), Some("openai"));
}

#[test]
fn map_stream_error_to_exception_keeps_the_body_fields() {
    let class = content_policy();
    assert!(matches!(class, ErrorClass::ContentPolicyViolation(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.code, "400");
    assert_eq!(
        metadata.error_type,
        WireField::Text("invalid_request_error".to_owned())
    );
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(
        metadata.openai_code.as_deref(),
        Some("content_policy_violation")
    );
}

#[test]
fn map_stream_error_to_exception_maps_a_server_error() {
    let class = mapped(&[("message", text("boom")), ("code", text("server_error"))]);
    assert!(matches!(class, ErrorClass::InternalServer(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 500);
    assert_eq!(
        metadata.error_type,
        WireField::Text("internal_server_error".to_owned())
    );
    assert_eq!(metadata.openai_code.as_deref(), Some("server_error"));
}

#[test]
fn map_stream_error_to_exception_maps_a_vector_store_timeout() {
    let class = mapped(&[
        ("message", text("late")),
        ("code", text("vector_store_timeout")),
    ]);
    assert!(matches!(class, ErrorClass::Timeout(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 504);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.openai_code, None);
}

#[test]
fn map_stream_error_to_exception_maps_numeric_status_codes() {
    let bad_request = mapped(&[("code", text("400"))]);
    assert!(matches!(bad_request, ErrorClass::BadRequest(_)));
    let bad_request_metadata = bad_request.metadata();
    assert_eq!(bad_request_metadata.status_code, 400);
    assert_eq!(bad_request_metadata.openai_code.as_deref(), Some("400"));
    let authentication = mapped(&[("code", text("401"))]);
    assert!(matches!(authentication, ErrorClass::Authentication(_)));
    assert_eq!(authentication.metadata().status_code, 401);
    let not_found = mapped(&[("code", text("404"))]);
    assert!(matches!(not_found, ErrorClass::NotFound(_)));
    assert_eq!(not_found.metadata().status_code, 404);
}

#[test]
fn map_stream_error_to_exception_maps_an_unknown_code_to_internal_server() {
    let class = mapped(&[("message", text("?")), ("code", text("mystery"))]);
    assert!(matches!(class, ErrorClass::InternalServer(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 500);
    assert_eq!(metadata.openai_code.as_deref(), Some("mystery"));
}

#[test]
fn a_content_policy_violation_falls_back_even_at_400() {
    let class = content_policy();
    assert!(mid_stream_fallback_eligible(&class, Some(400)));
}

#[rstest]
#[case::a_missing_status(None, true)]
#[case::a_rate_limit(Some(429), true)]
#[case::an_internal_server(Some(500), true)]
#[case::a_service_unavailable(Some(503), true)]
#[case::a_bad_request(Some(400), false)]
#[case::an_authentication_error(Some(401), false)]
#[case::a_not_found(Some(404), false)]
fn mid_stream_fallback_follows_the_status(#[case] status: Option<u16>, #[case] expected: bool) {
    let class = mapped(&[("message", text("boom"))]);
    assert_eq!(mid_stream_fallback_eligible(&class, status), expected);
}

#![cfg(test)]

use betterlite_error::dispatch::RawExceptionBody;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use rstest::rstest;

use super::{ErrorBodyKind, ProviderFailure, fallback_type, provider_failure, sdk_body};
use crate::chat::RequestError;
use crate::chat::execution::ChatError;

fn expected(status: u16, error_type: &str, param: Option<&str>) -> ProviderFailure {
    ProviderFailure {
        status,
        error_type: error_type.to_owned(),
        param: param.map(str::to_owned),
    }
}

#[rstest]
#[case::unauthorized(401, "authentication_error")]
#[case::forbidden(403, "permission_error")]
#[case::throttled(429, "rate_limit_error")]
#[case::last_client_status(499, "invalid_request_error")]
#[case::first_server_status(500, "internal_server_error")]
#[case::gateway(502, "internal_server_error")]
#[case::other_client_status(418, "invalid_request_error")]
fn fallback_type_names_each_status_family(#[case] status: u16, #[case] error_type: &str) {
    assert_eq!(fallback_type(status), error_type);
}

#[rstest]
#[case::bad_request_keeps_type_and_param(
    400,
    r#"{"error":{"message":"bad","type":"invalid_request_error","param":"temperature"}}"#,
    ErrorBodyKind::Structured,
    expected(400, "invalid_request_error", Some("temperature"))
)]
#[case::message_only_drops_the_body_fields(
    400,
    r#"{"error":{"message":"bad","type":"t","param":"temperature"}}"#,
    ErrorBodyKind::MessageOnly,
    expected(400, "invalid_request_error", None)
)]
#[case::type_and_param_none_take_the_fallback(
    400,
    r#"{"error":{"type":"None","param":"None"}}"#,
    ErrorBodyKind::Structured,
    expected(400, "invalid_request_error", None)
)]
#[case::status_429_throttles(
    429,
    r#"{"error":{"type":"requests","param":"p"}}"#,
    ErrorBodyKind::Structured,
    expected(429, "throttling_error", Some("p"))
)]
#[case::rate_limit_text_on_a_503(
    503,
    "rate-limit",
    ErrorBodyKind::Structured,
    expected(429, "throttling_error", None)
)]
#[case::wrong_key(
    401,
    r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","param":"key"}}"#,
    ErrorBodyKind::Structured,
    expected(401, "authentication_error", None)
)]
#[case::forbidden(
    403,
    r#"{"type":"t","param":"p"}"#,
    ErrorBodyKind::Structured,
    expected(403, "permission_error", None)
)]
#[case::timeout_preamble(
    500,
    r#"{"error":{"message":"Request timed out"}}"#,
    ErrorBodyKind::Structured,
    expected(408, "invalid_request_error", None)
)]
#[case::status_outside_the_client_range_clamps_to_500(
    304,
    "",
    ErrorBodyKind::Structured,
    expected(500, "internal_server_error", None)
)]
fn provider_failure_renders_the_dispatch_class(
    #[case] status: u16,
    #[case] body: &str,
    #[case] error_body: ErrorBodyKind,
    #[case] rendered: ProviderFailure,
) {
    let response = UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("built-in patterns compile");
    assert_eq!(provider_failure(&response, &redactor, error_body), rendered);
}

#[rstest]
#[case::error_object(
    br#"{"error":{"type":"invalid_request_error","param":"messages","code":"invalid","message":"bad"}}"#,
    Some(RawExceptionBody {
        kind: Some("invalid_request_error".to_owned()),
        param: Some("messages".to_owned()),
        code: Some("invalid".to_owned()),
        message: Some("bad".to_owned()),
        error: None,
    })
)]
#[case::whole_body_without_an_error_object(
    br#"{"type":"t","message":"m"}"#,
    Some(RawExceptionBody {
        kind: Some("t".to_owned()),
        param: None,
        code: None,
        message: Some("m".to_owned()),
        error: None,
    })
)]
#[case::not_json(b"not json", None)]
#[case::an_array_is_not_an_object(b"[]", None)]
fn sdk_body_reads_the_error_object_or_the_whole_body(
    #[case] bytes: &[u8],
    #[case] expected: Option<RawExceptionBody>,
) {
    assert_eq!(sdk_body(bytes), expected);
}

#[rstest]
#[case::unknown_model(ChatError::UnknownModel { model: "m".to_owned() }, 400, "invalid_request_error")]
#[case::bad_request(ChatError::from(RequestError::Unsupported { name: "x".to_owned() }), 400, "invalid_request_error")]
#[case::streaming(ChatError::Streaming, 501, "internal_server_error")]
#[case::unsupported_provider(ChatError::UnsupportedProvider { model: "m".to_owned() }, 501, "internal_server_error")]
#[case::missing_key(ChatError::MissingApiKey, 401, "authentication_error")]
#[case::timeout(ChatError::Timeout, 408, "invalid_request_error")]
#[case::too_large(ChatError::TooLarge, 500, "internal_server_error")]
fn maps_betterlite_errors_to_litellm_codes(
    #[case] error: ChatError,
    #[case] status: u16,
    #[case] error_type: &str,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("built-in patterns compile");
    assert_eq!(error.failure(&redactor), expected(status, error_type, None));
}

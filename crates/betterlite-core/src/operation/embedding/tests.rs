#![cfg(test)]

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::embedding::EmbeddingError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use rstest::rstest;

use super::{EmbeddingFailure, EmbeddingRoute, route_error};

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor")
}

fn provider(status: u16, body: &str) -> EmbeddingError {
    EmbeddingError::Provider {
        response: Box::new(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }),
    }
}

#[rstest]
#[case::unknown_model(
    EmbeddingError::UnknownModel { model: String::from("m") },
    400,
    "invalid_request_error",
    "no model_list entry is named m"
)]
#[case::unsupported_parameter(
    EmbeddingError::Unsupported { name: String::from("dimensions") },
    400,
    "invalid_request_error",
    "betterlite does not support the dimensions request parameter yet"
)]
#[case::missing_key(
    EmbeddingError::MissingApiKey,
    401,
    "authentication_error",
    "no API key is set for this deployment, in litellm_settings or in OPENAI_API_KEY"
)]
#[case::unsupported_provider(
    EmbeddingError::UnsupportedProvider { model: String::from("m") },
    501,
    "internal_server_error",
    "betterlite does not serve m embeddings yet"
)]
#[case::decode(
    EmbeddingError::Decode { source: serde_json::from_str::<u8>("x").unwrap_err() },
    500,
    "internal_server_error",
    "the provider answer is not an embeddings response: "
)]
fn embedding_errors_map_to_their_status_and_type(
    #[case] error: EmbeddingError,
    #[case] status: u16,
    #[case] error_type: &str,
    #[case] message: &str,
) {
    let route = route_error(&error, &redactor());
    assert_eq!(route.failure.status, status);
    assert_eq!(route.failure.error_type, error_type);
    assert_eq!(route.failure.param, None);
    assert!(route.message.starts_with(message), "{}", route.message);
    assert!(route.upstream_headers.is_none());
}

#[rstest]
#[case::with_param_and_code(
    r#"{"error":{"message":"bad input","type":"requests","param":"input","code":"c1"}}"#,
    "Error code: 429 - {'error': {'message': 'bad input', 'type': 'requests', 'param': 'input', 'code': 'c1'}}"
)]
#[case::without_param_and_code(
    r#"{"error":{"message":"bad input","type":"requests"}}"#,
    "Error code: 429 - {'error': {'message': 'bad input', 'type': 'requests', 'param': None, 'code': None}}"
)]
#[case::not_an_error_body("plain text", "the provider answered with status 429")]
fn a_provider_error_keeps_the_upstream_message(#[case] body: &str, #[case] message: &str) {
    let route = route_error(&provider(429, body), &redactor());
    assert_eq!(route.message, message);
    assert_eq!(route.failure.status, 429);
    assert!(route.upstream_headers.is_some());
}

#[rstest]
#[case::route(
    EmbeddingFailure::Route(EmbeddingError::MissingApiKey),
    401,
    "no API key is set for this deployment, in litellm_settings or in OPENAI_API_KEY"
)]
#[case::internal(
    EmbeddingFailure::Internal {
        message: String::from("boom\nTraceback (most recent call last):\n  file x"),
    },
    500,
    "boom"
)]
#[case::mismatched(
    EmbeddingFailure::Mismatched,
    500,
    "the provider answered a different operation"
)]
fn a_failure_routes_to_its_answer(
    #[case] failure: EmbeddingFailure,
    #[case] status: u16,
    #[case] message: &str,
) {
    let route = failure.route(&redactor());
    assert_eq!(route.failure.status, status);
    assert_eq!(route.message, message);
}

#[test]
fn an_internal_route_is_a_500_with_no_headers() {
    let route = EmbeddingRoute::internal(String::from("failed"));
    assert_eq!(route.failure.status, 500);
    assert_eq!(route.failure.error_type, "internal_server_error");
    assert_eq!(route.failure.param, None);
    assert_eq!(route.message, "failed");
    assert!(route.upstream_headers.is_none());
}

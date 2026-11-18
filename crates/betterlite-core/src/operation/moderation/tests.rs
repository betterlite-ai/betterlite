#![cfg(test)]

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_openai::chat::deployment::DeploymentError;
use betterlite_openai::moderation_execution::ModerationError;
use betterlite_openai::moderation_request::ModerationRequestError;
use betterlite_openai::moderation_response::ModerationResponseError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use rstest::rstest;

use super::route_error;

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor")
}

fn provider(status: u16, body: &str) -> ModerationError {
    ModerationError::Provider {
        response: Box::new(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }),
    }
}

fn json_error() -> serde_json::Error {
    serde_json::from_str::<u8>("x").expect_err("parse error")
}

// proxy_server.py's moderation handler reads the SDK exception's type and param, and answers str(status_code) as the code
#[rstest]
#[case::upstream_400(
    provider(
        400,
        r#"{"error":{"message":"Invalid input","type":"invalid_request_error","param":"input","code":"invalid_input"}}"#,
    ),
    400,
    Some("invalid_request_error"),
    Some("input"),
    Some("400")
)]
#[case::upstream_401(
    provider(
        401,
        r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","code":"invalid_api_key"}}"#,
    ),
    401,
    Some("invalid_request_error"),
    None,
    Some("401")
)]
#[case::upstream_429(
    provider(
        429,
        r#"{"error":{"message":"Rate limit reached for requests","type":"tokens","code":"rate_limit_exceeded"}}"#,
    ),
    429,
    Some("tokens"),
    None,
    Some("429")
)]
#[case::upstream_500(
    provider(500, r#"{"error":{"message":"The server had an error"}}"#),
    500,
    None,
    None,
    Some("500")
)]
#[case::flat_error_body(
    provider(
        400,
        r#"{"message":"bad","type":"tensors","param":"input","code":"c1"}"#
    ),
    400,
    Some("tensors"),
    Some("input"),
    Some("400")
)]
#[case::unparseable_body(provider(400, "plain text"), 400, None, None, Some("400"))]
#[case::body(
    ModerationError::Request(ModerationRequestError::Body { source: json_error() }),
    400,
    Some("invalid_request_error"),
    None,
    None
)]
#[case::unsupported(
    ModerationError::Request(ModerationRequestError::Unsupported { name: String::from("policy") }),
    400,
    Some("invalid_request_error"),
    None,
    None
)]
#[case::encode(
    ModerationError::Request(ModerationRequestError::Encode { source: json_error() }),
    500,
    Some("internal_server_error"),
    None,
    None
)]
#[case::decimal(
    ModerationError::Response(ModerationResponseError::Decimal {
        category: String::from("hate"),
        source: parse_decimal_lexeme("1e-29").expect_err("out of range"),
    }),
    502,
    Some("bad_gateway"),
    Some("category_scores"),
    Some("decimal_out_of_range")
)]
#[case::decode(
    ModerationError::Response(ModerationResponseError::Decode { source: json_error() }),
    500,
    Some("internal_server_error"),
    None,
    None
)]
#[case::missing_key(
    ModerationError::MissingApiKey,
    401,
    Some("authentication_error"),
    None,
    None
)]
#[case::unsupported_provider(
    ModerationError::UnsupportedProvider { model: String::from("bedrock/guard") },
    501,
    Some("internal_server_error"),
    None,
    None
)]
#[case::endpoint(
    ModerationError::Endpoint {
        source: DeploymentError::ApiBase {
            model_name: String::from("m"),
            source: url::ParseError::EmptyHost,
        },
    },
    500,
    Some("internal_server_error"),
    None,
    None
)]
#[case::timeout(
    ModerationError::Http {
        source: HttpError::Timeout,
    },
    408,
    None,
    None,
    None
)]
#[case::transport(
    ModerationError::Http {
        source: HttpError::TooLarge { limit: 1024 },
    },
    500,
    None,
    None,
    None
)]
fn moderations_moderation_tests_behavior(
    #[case] error: ModerationError,
    #[case] status: u16,
    #[case] error_type: Option<&str>,
    #[case] param: Option<&str>,
    #[case] code: Option<&str>,
) {
    let failure = route_error(&error, &redactor());
    assert_eq!(failure.status, status);
    assert_eq!(failure.error_type.as_deref(), error_type);
    assert_eq!(failure.param.as_deref(), param);
    assert_eq!(failure.code.as_deref(), code);
    assert!(!failure.message.is_empty(), "the failure carries a message");
}

#[test]
fn a_redacted_traceback_never_reaches_the_failure_message() {
    let error = ModerationError::Http {
        source: HttpError::Timeout,
    };
    let failure = route_error(&error, &redactor());
    assert!(
        !failure.message.contains("Traceback"),
        "{}",
        failure.message
    );
}

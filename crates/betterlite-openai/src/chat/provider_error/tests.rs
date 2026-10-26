#![cfg(test)]

use rstest::rstest;

use super::{ProviderFailure, classify};
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
#[case::bad_request_keeps_type_and_param(
    400,
    r#"{"error":{"message":"bad","type":"invalid_request_error","param":"temperature"}}"#,
    400,
    "invalid_request_error",
    Some("temperature")
)]
#[case::invalid_request_keeps_the_provider_status(
    409,
    r#"{"error":{"type":"invalid_request_error","param":"x"}}"#,
    409,
    "invalid_request_error",
    Some("x")
)]
#[case::wrong_key_is_an_authentication_error(401, r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","param":"key"}}"#, 401, "authentication_error", None)]
#[case::status_429_throttles(
    429,
    r#"{"error":{"type":"requests","param":"p"}}"#,
    429,
    "throttling_error",
    Some("p")
)]
#[case::rate_limit_words_throttle(
    400,
    r#"{"error":{"message":"Rate  Limit hit"}}"#,
    429,
    "throttling_error",
    None
)]
#[case::rate_limit_with_dash(503, "rate-limit", 429, "throttling_error", None)]
#[case::rate_limit_with_underscore(503, "rate_limit_exceeded", 429, "throttling_error", None)]
#[case::service_tier_capacity(503, "Service tier capacity exceeded", 429, "throttling_error", None)]
#[case::rate_without_limit_is_not_throttling(
    503,
    "rate unknown",
    503,
    "internal_server_error",
    None
)]
#[case::context_window(400, r#"{"error":{"message":"This model's maximum context length is 8","type":"invalid_request_error","param":"messages"}}"#, 400, "invalid_request_error", None)]
#[case::context_window_on_a_server_error(
    500,
    "input tokens exceed the configured limit",
    500,
    "internal_server_error",
    None
)]
#[case::cerebras_context(
    400,
    r#"{"type":"t","param":"p","message":"current length is 9 while limit is 8"}"#,
    400,
    "invalid_request_error",
    None
)]
#[case::maximum_input_length(
    400,
    r#"{"type":"t","param":"p","message":"maximum input length is 8 tokens"}"#,
    400,
    "invalid_request_error",
    None
)]
#[case::maximum_input_without_tokens(
    400,
    r#"{"type":"t","param":"p","message":"maximum input length is 8"}"#,
    400,
    "t",
    Some("p")
)]
#[case::length_without_limit(
    400,
    r#"{"type":"t","param":"p","message":"current length is 9"}"#,
    400,
    "t",
    Some("p")
)]
#[case::user_field_too_long_is_not_context(400, r#"{"type":"t","param":"user","message":"Invalid 'user': string too long. Expected a string with maximum length 64"}"#, 400, "t", Some("user"))]
#[case::user_named_in_a_context_error(400, r#"{"type":"t","param":"p","message":"invalid 'user': this model's maximum context length is 8"}"#, 400, "invalid_request_error", None)]
#[case::max_length_code_is_not_context(400, r#"{"type":"t","param":"p","code":"string_above_max_length","message":"string too long. expected a string with maximum length 9"}"#, 400, "t", Some("p"))]
#[case::model_not_found(
    400,
    r#"{"error":{"type":"invalid_request_error","code":"model_not_found","param":"model"}}"#,
    404,
    "invalid_request_error",
    None
)]
#[case::model_not_found_needs_invalid_request(
    400,
    r#"{"type":"t","param":"p","code":"model_not_found"}"#,
    400,
    "t",
    Some("p")
)]
#[case::timeout_text(500, "A timeout occurred", 408, "invalid_request_error", None)]
#[case::content_policy(403, r#"{"error":{"type":"policy","param":"prompt","message":"Invalid prompt: violating our usage policy"}}"#, 403, "policy", Some("prompt"))]
#[case::content_policy_code(403, r#"{"error":{"type":"invalid_request_error","code":"content_policy_violation","message":"Incorrect API key provided"}}"#, 403, "invalid_request_error", None)]
#[case::safety_system(403, r#"{"type":"t","param":"p","message":"Your request was rejected as a result of the safety system"}"#, 403, "t", Some("p"))]
#[case::invalid_prompt_alone(
    403,
    r#"{"type":"t","param":"p","message":"Invalid prompt"}"#,
    403,
    "permission_error",
    None
)]
#[case::usage_policy_alone(
    403,
    r#"{"type":"t","param":"p","message":"violating our usage policy"}"#,
    403,
    "permission_error",
    None
)]
#[case::encrypted_content(
    403,
    r#"{"type":"t","param":"p","code":"invalid_encrypted_content"}"#,
    403,
    "t",
    Some("p")
)]
#[case::unverified_content(
    403,
    r#"{"type":"t","message":"could not be verified"}"#,
    403,
    "t",
    None
)]
#[case::unknown_server_error(
    503,
    r#"{"param":"p","message":"Web server is returning an unknown error"}"#,
    500,
    "internal_server_error",
    Some("p")
)]
#[case::server_error_text(
    503,
    "The server had an error processing your request.",
    500,
    "internal_server_error",
    None
)]
#[case::request_too_large(
    413,
    r#"{"param":"p","message":"Request too large"}"#,
    429,
    "throttling_error",
    Some("p")
)]
#[case::unprocessable_keeps_type(422, r#"{"type":"t","param":"p"}"#, 422, "t", Some("p"))]
#[case::bad_request_without_type(400, "not json", 400, "invalid_request_error", None)]
#[case::param_none_is_null(400, r#"{"type":"t","param":"None"}"#, 400, "t", None)]
#[case::internal_error(
    500,
    r#"{"error":{"type":"server_error","param":"p"}}"#,
    500,
    "internal_server_error",
    Some("p")
)]
#[case::forbidden(403, r#"{"type":"t","param":"p"}"#, 403, "permission_error", None)]
#[case::not_found(404, r#"{"type":"t","param":"p"}"#, 404, "invalid_request_error", None)]
#[case::bad_gateway(502, r#"{"type":"t","param":"p"}"#, 502, "internal_server_error", None)]
#[case::unknown_client_status(418, "teapot", 418, "invalid_request_error", None)]
#[case::status_499(499, "", 499, "invalid_request_error", None)]
#[case::status_500_without_text(504, "", 504, "internal_server_error", None)]
fn maps_provider_errors_like_litellm(
    #[case] status: u16,
    #[case] body: &str,
    #[case] mapped_status: u16,
    #[case] error_type: &str,
    #[case] param: Option<&str>,
) {
    assert_eq!(
        classify(status, body),
        expected(mapped_status, error_type, param)
    );
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
    assert_eq!(error.failure(), expected(status, error_type, None));
}

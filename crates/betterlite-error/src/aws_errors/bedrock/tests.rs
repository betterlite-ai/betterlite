#![cfg(test)]

use rstest::rstest;

use super::*;

fn input(
    custom_llm_provider: &str,
    error_str: &str,
    status_code: Option<u16>,
) -> BedrockExceptionInput {
    BedrockExceptionInput {
        model: "claude-3".to_owned(),
        custom_llm_provider: custom_llm_provider.to_owned(),
        error_str: error_str.to_owned(),
        status_code,
    }
}

// RateLimit and Timeout nest the shared fields under "context"; every other family keeps them at the top level
macro_rules! provider_of {
    ($value:expr) => {
        $value["provider"]
            .as_str()
            .or_else(|| $value["context"]["provider"].as_str())
    };
}

#[rstest]
// neither "invalid_request_error" nor "validation_error" is present: the mantle guard returns before the regex runs
#[case::branch_863_mantle_guard_neither_keyword(
    "bedrock_mantle",
    "an unrelated failure",
    None,
    None,
    None
)]
// the keyword is present but the token-count regex never matches: the second mantle guard returns
#[case::branch_866_mantle_guard_regex_no_match(
    "bedrock_mantle",
    "invalid_request_error: nothing numeric here",
    None,
    None,
    None
)]
// a non-mantle provider skips the mantle block entirely and falls straight into the shared elif chain
#[case::branch_882_non_mantle_provider_skips_mantle_block(
    "bedrock",
    "too many tokens were supplied",
    None,
    Some("context_window_exceeded"),
    Some("bedrock")
)]
// both mantle guards pass: the structured validation error carries a matching token-count message
#[case::branch_884_mantle_positive_match(
    "bedrock_mantle",
    "invalid_request_error: prompt tokens (500) exceed model maximum (400)",
    None,
    Some("context_window_exceeded"),
    Some("bedrock_mantle")
)]
// the "validation_error" keyword alone satisfies the first mantle guard just as "invalid_request_error" does
#[case::mantle_validation_error_keyword(
    "bedrock_mantle",
    "validation_error: prompt tokens (500) exceed model maximum (400)",
    None,
    Some("context_window_exceeded"),
    Some("bedrock_mantle")
)]
#[case::branch_891_context_window_text(
    "bedrock",
    "too many tokens were supplied",
    None,
    Some("context_window_exceeded"),
    Some("bedrock")
)]
#[case::branch_905_conversation_tool_blocks(
    "bedrock",
    "Conversation blocks and tool result blocks cannot be provided in the same turn.",
    None,
    Some("bad_request"),
    Some("bedrock")
)]
#[case::branch_912_malformed_input(
    "bedrock",
    "Malformed input request body",
    None,
    Some("bad_request"),
    Some("bedrock")
)]
#[case::branch_919_conversation_must_start_user(
    "bedrock",
    "A conversation must start with a user message.",
    None,
    Some("bad_request"),
    Some("bedrock")
)]
#[case::branch_926_credentials_auth(
    "bedrock",
    "Unable to locate credentials for this request",
    None,
    Some("authentication"),
    Some("bedrock")
)]
#[case::branch_936_access_denied(
    "bedrock",
    "AccessDeniedException: no permission",
    None,
    Some("permission_denied"),
    Some("bedrock")
)]
#[case::branch_943_throttling(
    "bedrock",
    "ThrottlingException: slow down",
    None,
    Some("rate_limit"),
    Some("bedrock")
)]
#[case::branch_950_connect_timeout(
    "bedrock",
    "Connect timeout on endpoint URL",
    None,
    Some("timeout"),
    Some("bedrock")
)]
// no status_code at all: getattr(original_exception, "status_code", 500) defaults to 500 and the predicate matches
#[case::branch_957_image_processing_default_status(
    "bedrock",
    "Could not process image data",
    None,
    Some("internal_server"),
    Some("bedrock")
)]
// a present, non-500 status_code skips the image-processing predicate and falls through to the status branch below
#[case::could_not_process_image_explicit_other_status_falls_through(
    "bedrock_edge",
    "Could not process image data",
    Some(503),
    Some("service_unavailable"),
    Some("bedrock_edge")
)]
// hasattr(original_exception, "status_code") is false: nothing in the elif chain matches and the function returns None
#[case::branch_964_hasattr_false_falls_through("bedrock", "an unmatched failure", None, None, None)]
#[case::branch_965_status_500(
    "bedrock",
    "an unmatched failure",
    Some(500),
    Some("service_unavailable"),
    Some("bedrock")
)]
#[case::branch_972_status_401(
    "bedrock",
    "an unmatched failure",
    Some(401),
    Some("authentication"),
    Some("bedrock")
)]
#[case::branch_979_status_400(
    "bedrock",
    "an unmatched failure",
    Some(400),
    Some("bad_request"),
    Some("bedrock")
)]
#[case::branch_986_status_404(
    "bedrock",
    "an unmatched failure",
    Some(404),
    Some("not_found"),
    Some("bedrock")
)]
// 408/422/429/503/504 forward the caller's custom_llm_provider instead of the literal "bedrock"
#[case::branch_993_status_408_custom_provider(
    "my_bedrock_proxy",
    "an unmatched failure",
    Some(408),
    Some("timeout"),
    Some("my_bedrock_proxy")
)]
#[case::branch_1001_status_422_custom_provider(
    "my_bedrock_proxy",
    "an unmatched failure",
    Some(422),
    Some("bad_request"),
    Some("my_bedrock_proxy")
)]
#[case::branch_1009_status_429_custom_provider(
    "my_bedrock_proxy",
    "an unmatched failure",
    Some(429),
    Some("rate_limit"),
    Some("my_bedrock_proxy")
)]
#[case::branch_1017_status_503_custom_provider(
    "my_bedrock_proxy",
    "an unmatched failure",
    Some(503),
    Some("service_unavailable"),
    Some("my_bedrock_proxy")
)]
#[case::branch_1025_status_504_custom_provider(
    "my_bedrock_proxy",
    "an unmatched failure",
    Some(504),
    Some("timeout"),
    Some("my_bedrock_proxy")
)]
#[case::generic_fallback_unmatched_status("bedrock", "an unmatched failure", Some(418), None, None)]
fn aws_errors_behavior_tests_behavior(
    #[case] custom_llm_provider: &str,
    #[case] error_str: &str,
    #[case] status_code: Option<u16>,
    #[case] expected_family: Option<&str>,
    #[case] expected_provider: Option<&str>,
) {
    let mapped = map_bedrock_exception(&input(custom_llm_provider, error_str, status_code));
    match expected_family {
        None => assert!(mapped.is_none()),
        Some(family) => {
            let class = mapped.expect("a matching predicate must produce a class");
            let value = serde_json::to_value(&class).expect("ErrorClass always serializes");
            assert_eq!(value["family"], serde_json::json!(family));
            let expected =
                expected_provider.expect("a matched case always carries an expected provider");
            assert_eq!(provider_of!(value), Some(expected));
        }
    }
}

#[test]
fn message_carries_the_family_specific_reason() {
    let mapped = map_bedrock_exception(&input(
        "bedrock",
        "AccessDeniedException: no permission",
        None,
    ))
    .expect("AccessDeniedException matches the permission_denied predicate");
    let value = serde_json::to_value(&mapped).expect("ErrorClass always serializes");
    assert_eq!(
        value["message"],
        serde_json::json!("bedrock: the provider denied permission for the request")
    );
}

#[test]
fn text_predicate_wins_over_a_competing_status_code() {
    let mapped = map_bedrock_exception(&input(
        "bedrock",
        "too many tokens were supplied",
        Some(500),
    ))
    .expect("the text predicate matches before status_code is ever consulted");
    let value = serde_json::to_value(&mapped).expect("ErrorClass always serializes");
    assert_eq!(
        value["family"],
        serde_json::json!("context_window_exceeded")
    );
    assert_eq!(provider_of!(value), Some("bedrock"));
}

#![cfg(test)]

use rstest::rstest;

use super::status_failure;

#[rstest]
#[case::gateway_timeout(504, String::new(), "HTTP 504: ".to_owned(), true, true)]
#[case::service_unavailable(
    503,
    "down".to_owned(),
    "HTTP 503: down".to_owned(),
    true,
    true
)]
#[case::bad_gateway(
    502,
    "bad gateway".to_owned(),
    "HTTP 502: bad gateway".to_owned(),
    true,
    true
)]
#[case::server_error(
    500,
    "boom".to_owned(),
    "HTTP 500: boom".to_owned(),
    false,
    true
)]
#[case::rate_limited(
    429,
    "slow down".to_owned(),
    "HTTP 429: slow down".to_owned(),
    false,
    true
)]
#[case::request_timeout(
    408,
    "timeout".to_owned(),
    "HTTP 408: timeout".to_owned(),
    false,
    true
)]
#[case::bad_request(400, "bad".to_owned(), "HTTP 400: bad".to_owned(), false, false)]
#[case::not_found(
    404,
    "missing".to_owned(),
    "HTTP 404: missing".to_owned(),
    false,
    false
)]
#[case::truncates_the_body_to_two_hundred_characters(
    400,
    "x".repeat(250),
    format!("HTTP 400: {}", "x".repeat(200)),
    false,
    false
)]
#[case::truncates_by_characters_not_bytes(
    400,
    "\u{e9}".repeat(250),
    format!("HTTP 400: {}", "\u{e9}".repeat(200)),
    false,
    false
)]
fn straiker_webhook_context_tests_behavior(
    #[case] status: u16,
    #[case] text: String,
    #[case] expected_message: String,
    #[case] is_unreachable: bool,
    #[case] retryable: bool,
) {
    let failure = status_failure(status, &text);
    assert_eq!(failure.message, expected_message);
    assert_eq!(failure.is_unreachable, is_unreachable);
    assert_eq!(failure.retryable, retryable);
}

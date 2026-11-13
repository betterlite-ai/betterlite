#![cfg(test)]

use rstest::rstest;

use crate::s3_transport::error_code::{TERMINAL_CODES, error_code, is_terminal};

fn xml(code: &str) -> String {
    format!("<?xml version=\"1.0\"?><Error><Code>{code}</Code><Message>rejected</Message></Error>")
}

#[rstest]
#[case::plain_error("<Error><Code>NoSuchKey</Code></Error>", Some("NoSuchKey"))]
#[case::first_code_wins(
    "<Error><Code>SlowDown</Code><Code>Next</Code></Error>",
    Some("SlowDown")
)]
#[case::code_after_a_broken_code("<Code>a<b</Code><Code>later</Code>", Some("later"))]
#[case::newline_inside_the_code("<Code>a\nb</Code>", Some("a\nb"))]
#[case::spaces_are_kept("<Code> spaces </Code>", Some(" spaces "))]
#[case::empty_code("<Error><Code></Code></Error>", None)]
#[case::missing_code("<Error><Message>rejected</Message></Error>", None)]
#[case::lowercase_element("<error><code>NoSuchKey</code></error>", None)]
#[case::code_with_no_closing_tag("<Error><Code>NoSuchKey</Error>", None)]
#[case::closing_tag_with_a_suffix("<Code>a</CodeX>", None)]
#[case::plain_text("the server said no", None)]
#[case::empty_body("", None)]
fn s3_error_codes_are_read_from_the_response_xml(
    #[case] body: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(error_code(body), expected);
}

#[rstest]
#[case::bad_digest("BadDigest")]
#[case::entity_too_large("EntityTooLarge")]
#[case::invalid_argument("InvalidArgument")]
#[case::invalid_digest("InvalidDigest")]
#[case::invalid_request("InvalidRequest")]
#[case::key_too_long("KeyTooLongError")]
#[case::malformed_xml("MalformedXML")]
fn terminal_codes_are_final_for_400_and_403(#[case] code: &str) {
    assert!(is_terminal(400, &xml(code)));
    assert!(is_terminal(403, &xml(code)));
}

#[rstest]
#[case::server_error(500, "EntityTooLarge")]
#[case::not_found(404, "EntityTooLarge")]
#[case::throttled(429, "SlowDown")]
#[case::unknown_code(400, "NoSuchKey")]
#[case::other_terminal_code(400, "AccessDenied")]
fn other_statuses_and_codes_are_not_terminal(#[case] status: u16, #[case] code: &str) {
    assert!(!is_terminal(status, &xml(code)));
}

#[rstest]
#[case::empty_body(400, "")]
#[case::non_xml(400, "the server said no")]
#[case::empty_code(403, "<Error><Code></Code></Error>")]
#[case::broken_code(403, "<Code>a<b</Code>")]
fn malformed_bodies_fail_safe_toward_retry(#[case] status: u16, #[case] body: &str) {
    assert!(!is_terminal(status, body));
}

#[test]
fn the_terminal_code_set_is_exactly_the_pinned_one() {
    assert_eq!(
        TERMINAL_CODES,
        [
            "BadDigest",
            "EntityTooLarge",
            "InvalidArgument",
            "InvalidDigest",
            "InvalidRequest",
            "KeyTooLongError",
            "MalformedXML",
        ]
    );
}

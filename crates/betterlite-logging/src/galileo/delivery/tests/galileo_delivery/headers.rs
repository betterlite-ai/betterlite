use rstest::rstest;
use secrecy::SecretString;

use super::super::super::headers::{Header, plain_headers, redact_headers};

fn header(name: &str, value: &str) -> Header {
    Header {
        name: name.to_owned(),
        value: SecretString::from(value),
    }
}

#[rstest]
#[case::api_key("Galileo-API-Key", "abcdefgh12345678wxyz", "abcdefgh...wxyz")]
#[case::bearer("Authorization", "Bearer abcdefghijklmnop", "Bearer a...mnop")]
#[case::lower_case_name("authorization", "Bearer abcdefghijklmnop", "Bearer a...mnop")]
#[case::thirteen_characters("Authorization", "1234567890abc", "12345678...0abc")]
#[case::twelve_characters("Authorization", "1234567890ab", "***")]
#[case::short("Galileo-API-Key", "key", "***")]
#[case::empty_value("Authorization", "", "")]
#[case::multibyte(
    "Authorization",
    "\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}",
    "\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}...\u{e9}\u{e9}\u{e9}\u{e9}"
)]
fn secret_headers_keep_eight_and_four_characters(
    #[case] name: &str,
    #[case] value: &str,
    #[case] expected: &str,
) {
    let redacted = redact_headers(&[header(name, value)]);
    assert_eq!(redacted, vec![(name.to_owned(), expected.to_owned())]);
}

#[test]
fn other_headers_pass_through_unchanged() {
    let redacted = redact_headers(&plain_headers(&[
        ("accept", "application/json"),
        ("Content-Type", "application/json"),
    ]));
    assert_eq!(
        redacted,
        vec![
            ("accept".to_owned(), "application/json".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ]
    );
}

#[test]
fn debug_output_of_a_header_never_shows_the_value() {
    let shown = format!("{:?}", header("Authorization", "Bearer super-secret-token"));
    assert!(!shown.contains("super-secret-token"), "{shown}");
}

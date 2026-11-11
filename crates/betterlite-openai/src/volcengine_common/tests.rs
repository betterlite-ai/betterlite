#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use super::{DEFAULT_BASE, VolcEngineError, base_url, headers};
use crate::openai_base_config::Headers;

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn header_map(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::absent(None, DEFAULT_BASE)]
#[case::given(Some("https://ark.example.test"), "https://ark.example.test")]
#[case::empty_used_as_given(Some(""), "")]
fn base_url_takes_the_given_base_or_the_default(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(base_url(api_base), expected);
}

#[test]
fn headers_carry_the_content_type_and_the_bearer_key() {
    let built = headers(&secret("volc-key"), &Headers::new());
    assert_eq!(
        built,
        header_map(&[
            ("Authorization", "Bearer volc-key"),
            ("Content-Type", "application/json"),
        ])
    );
}

#[rstest]
#[case::empty_key("", "Bearer ")]
#[case::plain_key("abc", "Bearer abc")]
fn headers_keep_an_empty_key_as_given(#[case] key: &str, #[case] expected: &str) {
    let built = headers(&secret(key), &Headers::new());
    assert_eq!(
        built.get("Authorization").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn extra_headers_go_on_top_and_win_on_the_same_key() {
    let extra = header_map(&[
        ("Content-Type", "text/plain"),
        ("Authorization", "Custom"),
        ("x-call", "1"),
    ]);
    let built = headers(&secret("volc-key"), &extra);
    assert_eq!(
        built,
        header_map(&[
            ("Authorization", "Custom"),
            ("Content-Type", "text/plain"),
            ("x-call", "1"),
        ])
    );
}

#[test]
fn the_error_keeps_the_status_the_message_and_the_headers() {
    let error = VolcEngineError {
        status_code: 429,
        message: String::from("rate limited"),
        headers: header_map(&[("retry-after", "2")]),
    };
    assert_eq!(error.status_code, 429);
    assert_eq!(error.to_string(), "rate limited");
    assert_eq!(
        error.headers.get("retry-after").map(String::as_str),
        Some("2")
    );
}

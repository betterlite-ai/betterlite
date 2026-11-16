#![cfg(test)]

use rstest::rstest;

use crate::api::video_contracts::config::BaseVideoConfig as _;
use crate::api::video_contracts::records::Headers;

use super::fixture::Subject;

#[rstest]
#[case::with_headers(
    429,
    "slow down",
    &[("retry-after", "2"), ("x-request-id", "abc")]
)]
#[case::without_headers(500, "boom", &[])]
fn error_class_carries_status_message_and_headers(
    #[case] status_code: u16,
    #[case] message: &str,
    #[case] pairs: &[(&str, &str)],
) {
    let headers: Headers = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let error = Subject.get_error_class(message, status_code, &headers);
    let sent: Vec<(String, String)> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(error.status_code, status_code);
    assert_eq!(error.message, message);
    assert_eq!(error.headers, Some(sent));
    assert_eq!(error.response.status_code, status_code);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.body, None);
}

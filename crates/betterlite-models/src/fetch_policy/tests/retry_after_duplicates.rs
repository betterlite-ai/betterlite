use super::support;
use crate::fetch_policy::response::FetchResponse;
use crate::fetch_policy::retry::parse_retry_after_seconds;

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let repeated = FetchResponse::new(
        200,
        vec![
            ("Retry-After".to_owned(), "3".to_owned()),
            ("retry-after".to_owned(), "4".to_owned()),
        ],
        Vec::new(),
    );
    assert_eq!(parse_retry_after_seconds(&repeated), None);

    let single = FetchResponse::new(
        200,
        vec![("Retry-After".to_owned(), "3".to_owned())],
        Vec::new(),
    );
    assert_eq!(
        parse_retry_after_seconds(&single),
        Some(support::decimal("3"))
    );
}

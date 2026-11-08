use std::collections::BTreeMap;

use rstest::rstest;

use super::support;
use crate::fetch_policy::response::FetchResponse;
use crate::fetch_policy::retry::parse_retry_after_seconds;

#[rstest]
#[case::absent(None, None)]
#[case::seconds(Some("3.5"), Some("3.5"))]
#[case::zero(Some("0"), Some("0"))]
#[case::negative(Some("-1"), None)]
#[case::non_numeric(Some("soon"), None)]
#[case::padded(Some(" 7 "), Some("7"))]
#[case::scientific(Some("1e3"), Some("1000"))]
#[case::fraction(Some("0.5"), Some("0.5"))]
fn catalog_fetch_policy_behavior_tests_behavior(
    #[case] header: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let headers = header.map_or_else(BTreeMap::new, |value| {
        BTreeMap::from([("Retry-After".to_owned(), value.to_owned())])
    });
    let response = FetchResponse::new(200, headers, Vec::new());
    assert_eq!(
        parse_retry_after_seconds(&response),
        expected.map(support::decimal)
    );
}

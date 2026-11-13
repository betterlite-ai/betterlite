use rstest::rstest;

use super::support;
use crate::fetch_policy::outcome::classify_fetch_response;

#[rstest]
#[case::array("[]")]
#[case::number("3")]
#[case::text("\"models\"")]
#[case::null("null")]
fn catalog_fetch_policy_behavior_tests_behavior(#[case] body: &str) {
    let unavailable = support::unavailable(classify_fetch_response(
        support::response(200, body),
        support::URL,
    ));
    assert_eq!(
        unavailable.reason,
        format!("expected a JSON object from {}", support::URL)
    );
}

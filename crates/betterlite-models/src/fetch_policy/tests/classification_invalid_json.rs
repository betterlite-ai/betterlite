use super::support;
use crate::fetch_policy::outcome::classify_fetch_response;

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let unavailable = support::unavailable(classify_fetch_response(
        support::response(200, "{"),
        support::URL,
    ));
    assert!(
        unavailable
            .reason
            .starts_with("invalid JSON from https://catalog.test/model.json")
    );
}

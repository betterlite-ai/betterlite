#![cfg(test)]
use crate::batch::selection::merge_deployment_params;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

#[rstest::rstest]
#[case::deployment_params_override_request(false)]
#[case::nested_request_overrides_deployment(true)]
#[test]
fn batch_calls_batch_call_tests_behavior(#[case] nested: bool) {
    let request = IndexMap::from([("api_base".to_owned(), SecretString::from("https://shared"))]);
    let first = IndexMap::from([
        ("api_key".to_owned(), SecretString::from("first")),
        ("api_base".to_owned(), SecretString::from("https://first")),
    ]);
    let second = IndexMap::from([("api_base".to_owned(), SecretString::from("https://second"))]);
    let overrides = if nested {
        request.clone()
    } else {
        IndexMap::new()
    };
    let initial = merge_deployment_params(&request, &first, &overrides);
    let next = merge_deployment_params(&request, &second, &overrides);
    assert_eq!(initial.get("api_key").unwrap().expose_secret(), "first");
    assert!(!next.contains_key("api_key"));
    let expected = if nested {
        "https://shared"
    } else {
        "https://second"
    };
    assert_eq!(next.get("api_base").unwrap().expose_secret(), expected);
    assert_eq!(
        request.get("api_base").unwrap().expose_secret(),
        "https://shared"
    );
}

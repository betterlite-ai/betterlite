#![cfg(test)]

use super::header_map;

#[test]
fn signed_authorization_is_sensitive_and_other_headers_survive() {
    let headers = header_map(&[
        (
            "Authorization".to_owned(),
            "Bearer fixture-secret".to_owned(),
        ),
        ("x-custom".to_owned(), "value".to_owned()),
    ])
    .unwrap();
    assert!(headers.get("authorization").unwrap().is_sensitive());
    assert_eq!(headers.get("x-custom").unwrap(), "value");
    assert!(!format!("{headers:?}").contains("fixture-secret"));
}

#[rstest::rstest]
#[case::name("invalid header", "fixture-secret")]
#[case::value("Authorization", "fixture-secret\ninvalid")]
fn bad_headers_do_not_echo_values(#[case] name: &str, #[case] value: &str) {
    let error = header_map(&[(name.to_owned(), value.to_owned())]).unwrap_err();
    assert!(!format!("{error:?}: {error}").contains("fixture-secret"));
    assert!(error.to_string().contains(name));
}

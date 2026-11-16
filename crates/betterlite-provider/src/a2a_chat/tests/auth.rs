use betterlite_foundry::credentials::error::CredentialsError;
use rstest::rstest;

use crate::a2a_chat::auth::{a2a_hop_uses_entra, resolve_a2a_hop_auth_header};

use super::support::{FakeEntra, RefusingEntra, header_map, object};

#[rstest]
#[case::ad_token(r#"{"azure_ad_token":"t"}"#, None, true)]
#[case::empty_ad_token(r#"{"azure_ad_token":""}"#, None, false)]
#[case::null_ad_token(r#"{"azure_ad_token":null}"#, None, false)]
#[case::client_secret(r#"{"client_secret":"s"}"#, None, true)]
#[case::azure_password(r#"{"azure_password":"p"}"#, None, true)]
#[case::no_credentials("{}", None, false)]
#[case::named_provider(r#"{"client_secret":"s"}"#, Some("azure_ai"), false)]
#[case::empty_provider(r#"{"client_secret":"s"}"#, Some(""), true)]
fn a2a_hop_uses_entra_truth_table(
    #[case] params: &str,
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    let parsed = object(params);
    assert_eq!(a2a_hop_uses_entra(&parsed, provider), expected);
}

#[test]
fn a_hop_without_entra_has_no_auth_header() {
    let params = object(r#"{"api_key":"k"}"#);
    assert_eq!(
        resolve_a2a_hop_auth_header(&params, None, &FakeEntra).unwrap(),
        None
    );
    let entra = object(r#"{"azure_ad_token":"t"}"#);
    assert_eq!(
        resolve_a2a_hop_auth_header(&entra, Some("custom"), &FakeEntra).unwrap(),
        None
    );
}

#[test]
fn an_entra_hop_takes_the_resolver_header() {
    let params = object(r#"{"azure_ad_token":"t"}"#);
    let found = resolve_a2a_hop_auth_header(&params, None, &FakeEntra).unwrap();
    assert_eq!(
        found,
        Some(header_map(&[("Authorization", "Bearer header-token")]))
    );
}

#[test]
fn an_entra_header_failure_is_carried() {
    let params = object(r#"{"azure_ad_token":"t"}"#);
    let error = resolve_a2a_hop_auth_header(&params, None, &RefusingEntra).unwrap_err();
    assert!(matches!(
        error,
        CredentialsError::AgentTokenUnresolved { .. }
    ));
}

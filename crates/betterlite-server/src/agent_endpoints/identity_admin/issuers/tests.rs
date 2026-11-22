#![cfg(test)]

use betterlite_config::sections::jwt_settings::records::IssuerConfig;
use rstest::rstest;

use super::{IdentitySettings, trusted_agent_issuers};

fn entry(fields: &str) -> IssuerConfig {
    serde_json::from_str(fields).unwrap()
}

fn settings(
    entries: &[&str],
    jwt_issuer: Option<&str>,
    jwt_audience: Option<&str>,
) -> IdentitySettings {
    IdentitySettings {
        enable_jwt_auth: true,
        issuers: entries.iter().copied().map(entry).collect(),
        jwt_issuer: jwt_issuer.map(str::to_owned),
        jwt_audience: jwt_audience.map(str::to_owned),
        microsoft_tenant: None,
        microsoft_client_id: None,
    }
}

#[test]
fn jwt_auth_off_trusts_nothing() {
    let mut off = settings(
        &[r#"{"issuer":"https://kept","audience":"api"}"#],
        Some("https://global"),
        Some("api"),
    );
    off.enable_jwt_auth = false;
    assert!(trusted_agent_issuers(&off).is_empty());
}

#[test]
fn only_entries_with_a_non_empty_audience_and_validation_are_trusted() {
    let kept = settings(
        &[
            r#"{"issuer":"https://first","audience":"api"}"#,
            r#"{"issuer":"https://no-audience"}"#,
            r#"{"issuer":"https://empty-string","audience":""}"#,
            r#"{"issuer":"https://empty-list","audience":[]}"#,
            r#"{"issuer":"https://unvalidated","audience":"api","disable_audience_validation":true}"#,
            r#"{"issuer":"https://list","audience":["api","other"]}"#,
            r#"{"issuer":"https://list-with-blank","audience":["","api"]}"#,
        ],
        None,
        None,
    );
    assert_eq!(
        trusted_agent_issuers(&kept),
        vec![
            String::from("https://first"),
            String::from("https://list"),
            String::from("https://list-with-blank"),
        ]
    );
}

#[test]
fn duplicate_issuers_are_kept_in_order() {
    let duplicated = settings(
        &[
            r#"{"issuer":"https://same","audience":"api"}"#,
            r#"{"issuer":"https://same","audience":"other"}"#,
        ],
        None,
        None,
    );
    assert_eq!(
        trusted_agent_issuers(&duplicated),
        vec![String::from("https://same"), String::from("https://same")]
    );
}

#[test]
fn the_global_issuer_is_appended_last() {
    let with_global = settings(
        &[r#"{"issuer":"https://kept","audience":"api"}"#],
        Some("https://global"),
        Some("api"),
    );
    assert_eq!(
        trusted_agent_issuers(&with_global),
        vec![String::from("https://kept"), String::from("https://global")]
    );
}

#[rstest]
#[case::no_issuer(None, Some("api"))]
#[case::empty_issuer(Some(""), Some("api"))]
#[case::no_audience(Some("https://global"), None)]
#[case::empty_audience(Some("https://global"), Some(""))]
fn a_global_issuer_needs_both_halves(
    #[case] jwt_issuer: Option<&str>,
    #[case] jwt_audience: Option<&str>,
) {
    let half = settings(&[], jwt_issuer, jwt_audience);
    assert!(trusted_agent_issuers(&half).is_empty());
}

#[test]
fn a_global_issuer_a_dropped_entry_names_is_not_appended() {
    let dropped = settings(
        &[r#"{"issuer":"https://global"}"#],
        Some("https://global"),
        Some("api"),
    );
    assert!(trusted_agent_issuers(&dropped).is_empty());
}

#[test]
fn a_global_issuer_a_kept_entry_names_is_not_appended() {
    let already = settings(
        &[r#"{"issuer":"https://global","audience":"api"}"#],
        Some("https://global"),
        Some("api"),
    );
    assert_eq!(
        trusted_agent_issuers(&already),
        vec![String::from("https://global")]
    );
}

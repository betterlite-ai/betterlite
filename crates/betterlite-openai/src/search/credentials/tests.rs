#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::search::credentials::{CredentialError, ServerCredential};

const TAVILY: ServerCredential = ServerCredential {
    key_vars: &["TAVILY_API_KEY", "TAVILY_KEY"],
    base_var: Some("TAVILY_API_BASE"),
    default_base: Some("https://api.tavily.com/search"),
};

fn resolve(
    credential: &ServerCredential,
    caller_key: Option<&str>,
    caller_base: Option<&str>,
    environment: &[(&str, &str)],
) -> Result<Option<String>, CredentialError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let key = caller_key.map(SecretString::from);
    credential
        .resolve(key, caller_base, &lookup)
        .map(|resolved| resolved.map(|secret| secret.expose_secret().to_owned()))
}

#[rstest]
#[case::caller_key_skips_trust(Some("sk-caller"), Some("https://evil.example"), &[("TAVILY_API_KEY", "sk-server")], Some("sk-caller"))]
#[case::empty_caller_key_falls_back(Some(""), None, &[("TAVILY_API_KEY", "sk-server")], Some("sk-server"))]
#[case::first_var_wins(None, None, &[("TAVILY_API_KEY", "sk-first"), ("TAVILY_KEY", "sk-second")], Some("sk-first"))]
#[case::second_var_when_first_unset(None, None, &[("TAVILY_KEY", "sk-second")], Some("sk-second"))]
#[case::second_var_when_first_empty(None, None, &[("TAVILY_API_KEY", ""), ("TAVILY_KEY", "sk-second")], Some("sk-second"))]
#[case::no_server_key_is_keyless(None, Some("https://evil.example"), &[], None)]
#[case::no_caller_base(None, None, &[("TAVILY_API_KEY", "sk-server")], Some("sk-server"))]
#[case::empty_caller_base(None, Some(""), &[("TAVILY_API_KEY", "sk-server")], Some("sk-server"))]
fn credential_resolves_in_order(
    #[case] caller_key: Option<&str>,
    #[case] caller_base: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let resolved = resolve(&TAVILY, caller_key, caller_base, environment).unwrap();
    assert_eq!(resolved.as_deref(), expected);
}

#[test]
fn credential_without_key_vars_is_keyless() {
    let keyless = ServerCredential {
        key_vars: &[],
        base_var: None,
        default_base: None,
    };
    let resolved = resolve(&keyless, None, Some("https://searx.local"), &[]).unwrap();
    assert_eq!(resolved, None);
}

#[test]
fn credential_error_names_first_var_and_base() {
    let environment = [("TAVILY_KEY", "sk-server")];
    let error = resolve(&TAVILY, None, Some("https://evil.example/x"), &environment).unwrap_err();
    assert_eq!(
        error,
        CredentialError::UntrustedBase {
            credential: "TAVILY_API_KEY",
            api_base: String::from("https://evil.example/x"),
        }
    );
    assert!(!error.to_string().contains("sk-server"));
}

#[rstest]
#[case::default_host_other_path("https://API.Tavily.com/v2/custom", &[], true)]
#[case::other_scheme_same_netloc("http://api.tavily.com", &[], true)]
#[case::default_port_is_same_destination("https://api.tavily.com:443/search", &[], true)]
#[case::other_port("https://api.tavily.com:8443/search", &[], false)]
#[case::other_host("https://api.tavily.com.evil.example/search", &[], false)]
#[case::userinfo_changes_netloc("https://user@api.tavily.com/search", &[], false)]
#[case::operator_override("https://proxy.internal:9000/tavily", &[("TAVILY_API_BASE", "https://proxy.internal:9000")], true)]
#[case::empty_override_ignored("https://proxy.internal", &[("TAVILY_API_BASE", "")], false)]
#[case::no_scheme_has_no_host("api.tavily.com/search", &[], false)]
#[case::unparsable("http://[::1", &[], false)]
fn search_trust_compares_lowercase_netloc(
    #[case] caller_base: &str,
    #[case] extra_environment: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let mut environment = vec![("TAVILY_API_KEY", "sk-server")];
    environment.extend_from_slice(extra_environment);
    let resolved = resolve(&TAVILY, None, Some(caller_base), &environment);
    assert_eq!(resolved.is_ok(), trusted, "{resolved:?}");
}

#[test]
fn search_trust_without_default_or_override_refuses() {
    let bare = ServerCredential {
        key_vars: &["BRAVE_API_KEY"],
        base_var: None,
        default_base: None,
    };
    let environment = [("BRAVE_API_KEY", "sk-server")];
    let error = resolve(
        &bare,
        None,
        Some("https://api.search.brave.com"),
        &environment,
    );
    assert!(matches!(error, Err(CredentialError::UntrustedBase { .. })));
}

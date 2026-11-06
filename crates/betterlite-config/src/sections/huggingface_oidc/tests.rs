#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::EnvironmentLookup;
use crate::sections::Section as _;
use crate::sections::huggingface_oidc::{Raw, Settings};

struct TestEnvironment {
    values: BTreeMap<String, String>,
}

impl EnvironmentLookup for TestEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
    fn system_home(&self) -> Option<PathBuf> {
        None
    }
}

fn settings(entries: &[(&str, &str)]) -> Settings {
    let values = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Raw::default().convert(&TestEnvironment { values }).unwrap()
}

#[test]
fn settings_follow_huggingface_hub() {
    let defaults = settings(&[]);
    assert_eq!(defaults.resource, None);
    assert!(defaults.id_token.is_none());
    assert!(!defaults.github_actions);
    assert_eq!(defaults.request_url, None);
    assert!(defaults.request_token.is_none());
    assert_eq!(
        defaults.device_code_client_id,
        "26be6b09-91c5-47da-9861-d2d2bb7a7e36"
    );
    let configured = settings(&[
        ("HF_OIDC_RESOURCE", "org/repo"),
        ("HF_OIDC_ID_TOKEN", "id-token"),
        ("GITHUB_ACTIONS", "true"),
        (
            "ACTIONS_ID_TOKEN_REQUEST_URL",
            "https://actions.example/token",
        ),
        ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "request-token"),
        ("HF_DEVICE_CODE_OAUTH_CLIENT_ID", "client"),
    ]);
    assert_eq!(configured.resource.as_deref(), Some("org/repo"));
    assert_eq!(configured.id_token.unwrap().expose_secret(), "id-token");
    assert!(configured.github_actions);
    assert_eq!(
        configured.request_url.as_deref(),
        Some("https://actions.example/token")
    );
    assert_eq!(
        configured.request_token.unwrap().expose_secret(),
        "request-token"
    );
    assert_eq!(configured.device_code_client_id, "client");
}

#[rstest]
#[case::capitalized("True")]
#[case::one("1")]
#[case::empty("")]
fn only_the_exact_word_true_enables_github_actions(#[case] value: &str) {
    assert!(!settings(&[("GITHUB_ACTIONS", value)]).github_actions);
}

#[rstest]
#[case::resource("HF_OIDC_RESOURCE")]
#[case::id_token("HF_OIDC_ID_TOKEN")]
#[case::request_url("ACTIONS_ID_TOKEN_REQUEST_URL")]
#[case::request_token("ACTIONS_ID_TOKEN_REQUEST_TOKEN")]
fn empty_values_count_as_unset(#[case] name: &str) {
    let parsed = settings(&[(name, "")]);
    assert_eq!(parsed.resource, None);
    assert!(parsed.id_token.is_none());
    assert_eq!(parsed.request_url, None);
    assert!(parsed.request_token.is_none());
}

#[test]
fn empty_client_id_keeps_the_default() {
    assert_eq!(
        settings(&[("HF_DEVICE_CODE_OAUTH_CLIENT_ID", "")]).device_code_client_id,
        "26be6b09-91c5-47da-9861-d2d2bb7a7e36"
    );
}

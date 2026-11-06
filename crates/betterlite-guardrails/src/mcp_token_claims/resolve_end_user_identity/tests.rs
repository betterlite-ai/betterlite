#![cfg(test)]

use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use secrecy::SecretString;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use crate::mcp_token_claims::resolve_end_user_identity::resolve;
use crate::mcp_token_claims::{AuthFields, Claims};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let result = with_default(subscriber, run);
    let output = String::from_utf8(captured.0.lock().expect("poisoned").clone())
        .expect("tracing output is utf8");
    (result, output)
}

fn claims(json: &str) -> Claims {
    serde_json::from_str(json).unwrap()
}

fn sources(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn auth() -> AuthFields {
    AuthFields {
        user_id: None,
        user_email: None,
        end_user_id: None,
        team_id: None,
        org_id: None,
        token: None,
        api_key: None,
    }
}

#[test]
fn the_first_source_with_a_value_wins() {
    let incoming = claims(r#"{"sub":"alice","email":"alice@example.com"}"#);
    assert_eq!(
        resolve(
            &sources(&["token:sub", "token:email"]),
            &auth(),
            Some(&incoming)
        ),
        "alice"
    );
}

#[test]
fn an_empty_claim_falls_through_to_the_next_source() {
    let incoming = claims(r#"{"sub":"","email":"alice@example.com"}"#);
    assert_eq!(
        resolve(
            &sources(&["token:sub", "token:email"]),
            &auth(),
            Some(&incoming)
        ),
        "alice@example.com"
    );
}

#[test]
fn a_source_without_the_claim_falls_through_to_the_next_source() {
    let incoming = claims(r#"{"email":"alice@example.com"}"#);
    assert_eq!(
        resolve(
            &sources(&["token:sub", "token:email"]),
            &auth(),
            Some(&incoming)
        ),
        "alice@example.com"
    );
}

#[test]
fn litellm_user_id_resolves_the_authenticated_user() {
    let mut auth = auth();
    auth.user_id = Some("user-1".to_owned());
    assert_eq!(
        resolve(&sources(&["litellm:user_id"]), &auth, None),
        "user-1"
    );
}

#[test]
fn litellm_email_resolves_the_authenticated_email() {
    let mut auth = auth();
    auth.user_email = Some("alice@example.com".to_owned());
    assert_eq!(
        resolve(&sources(&["litellm:email"]), &auth, None),
        "alice@example.com"
    );
}

#[test]
fn litellm_end_user_id_resolves_the_end_user() {
    let mut auth = auth();
    auth.end_user_id = Some("end-user-1".to_owned());
    assert_eq!(
        resolve(&sources(&["litellm:end_user_id"]), &auth, None),
        "end-user-1"
    );
}

#[test]
fn litellm_team_id_resolves_the_team() {
    let mut auth = auth();
    auth.team_id = Some("team-1".to_owned());
    assert_eq!(
        resolve(&sources(&["litellm:team_id"]), &auth, None),
        "team-1"
    );
}

#[test]
fn an_empty_litellm_field_falls_through_to_the_next_source() {
    let mut auth = auth();
    auth.user_id = Some(String::new());
    let incoming = claims(r#"{"sub":"alice"}"#);
    assert_eq!(
        resolve(
            &sources(&["litellm:user_id", "token:sub"]),
            &auth,
            Some(&incoming)
        ),
        "alice"
    );
}

#[test]
fn an_unknown_source_warns_and_the_next_source_still_wins() {
    let incoming = claims(r#"{"sub":"alice"}"#);
    let (subject, logs) = capture(|| {
        resolve(
            &sources(&["litellm:org_id", "token:sub"]),
            &auth(),
            Some(&incoming),
        )
    });
    assert_eq!(subject, "alice");
    assert!(logs.contains("WARN"));
    assert!(logs.contains("litellm:org_id"));
}

#[test]
fn a_service_account_falls_back_to_a_hash_of_its_api_key() {
    let mut auth = auth();
    auth.token = Some(SecretString::from("sk-test-token"));
    assert_eq!(
        resolve(&sources(&["token:sub"]), &auth, None),
        "apikey:389faa3c31e10c88"
    );
}

#[test]
fn the_api_key_field_is_used_when_no_token_is_set() {
    let mut auth = auth();
    auth.api_key = Some(SecretString::from("plain-api-key"));
    assert_eq!(
        resolve(&sources(&[]), &auth, None),
        "apikey:aec3e83011b517c7"
    );
}

#[test]
fn an_empty_token_falls_back_to_the_api_key() {
    let mut auth = auth();
    auth.token = Some(SecretString::from(""));
    auth.api_key = Some(SecretString::from("plain-api-key"));
    assert_eq!(
        resolve(&sources(&[]), &auth, None),
        "apikey:aec3e83011b517c7"
    );
}

#[test]
fn without_a_credential_the_subject_is_the_proxy() {
    assert_eq!(resolve(&sources(&[]), &auth(), None), "litellm-proxy");
}

#[test]
fn the_api_key_hash_is_stable_and_never_logged() {
    let mut auth = auth();
    auth.token = Some(SecretString::from("sk-test-token"));
    let (first, logs) = capture(|| resolve(&sources(&["token:sub"]), &auth, None));
    let (second, _) = capture(|| resolve(&sources(&["token:sub"]), &auth, None));
    assert_eq!(first, "apikey:389faa3c31e10c88");
    assert_eq!(first, second);
    assert!(logs.is_empty(), "{logs}");
    assert!(!logs.contains("389faa3c31e10c88"));
}

#[rstest]
#[case::number(r#"{"sub":42}"#, "42")]
#[case::true_value(r#"{"sub":true}"#, "true")]
#[case::zero(r#"{"sub":0}"#, "apikey:aec3e83011b517c7")]
#[case::null_value(r#"{"sub":null}"#, "apikey:aec3e83011b517c7")]
#[case::list(r#"{"sub":["a"]}"#, "apikey:aec3e83011b517c7")]
#[case::object(r#"{"sub":{"a":1}}"#, "apikey:aec3e83011b517c7")]
fn non_text_claims_only_yield_scalar_identities(#[case] json: &str, #[case] expected: &str) {
    let mut auth = auth();
    auth.api_key = Some(SecretString::from("plain-api-key"));
    let incoming = claims(json);
    assert_eq!(
        resolve(&sources(&["token:sub"]), &auth, Some(&incoming)),
        expected
    );
}

#[test]
fn an_unknown_token_claim_falls_through() {
    let incoming = claims(r#"{"email":"alice@example.com"}"#);
    assert_eq!(
        resolve(
            &sources(&["token:unknown", "token:email"]),
            &auth(),
            Some(&incoming)
        ),
        "alice@example.com"
    );
}

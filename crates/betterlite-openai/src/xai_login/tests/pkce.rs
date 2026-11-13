use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::xai_login::error::XaiLoginError;
use crate::xai_login::pkce::{
    Entropy as _, OsEntropy, build_authorize_url, pkce_pair, pkce_pair_from,
};
use crate::xai_login::service::{RandomUuids, Uuids as _};

use super::support::{NONCE, STATE, TestEntropy};

const VERIFIER: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
const CHALLENGE: &str = "3Ev4DHdHPRMPoN6GukAY_pi7IUAF5qWJHRK6kURvnoE";
const ENDPOINT: &str = "https://auth.x.ai/oauth2/authorize";
const REDIRECT: &str = "http://127.0.0.1:56121/callback";
const URL: &str = "https://auth.x.ai/oauth2/authorize?response_type=code&client_id=b1a00492-073a-47ea-816f-4c329264a828&redirect_uri=http%3A%2F%2F127.0.0.1%3A56121%2Fcallback&scope=openid+profile+email+offline_access+grok-cli%3Aaccess+api%3Aaccess&code_challenge=3Ev4DHdHPRMPoN6GukAY_pi7IUAF5qWJHRK6kURvnoE&code_challenge_method=S256&state=state-test&nonce=nonce-test";

#[test]
fn fixed_entropy_yields_the_pinned_verifier_and_challenge() {
    let entropy = TestEntropy {
        bytes: [7; 32],
        fail: false,
    };
    let (verifier, challenge) = pkce_pair(&entropy).unwrap();
    assert_eq!(verifier.expose_secret(), VERIFIER);
    assert_eq!(challenge, CHALLENGE);
}

#[rstest]
#[case::single(&[0_u8; 1])]
#[case::full(&[0xff_u8; 32])]
fn the_verifier_and_challenge_are_unpadded_url_safe_base64(#[case] random: &[u8]) {
    let (verifier, challenge) = pkce_pair_from(random);
    let raw = verifier.expose_secret();
    assert!(!raw.contains('='));
    assert!(!challenge.contains('='));
    assert!(
        raw.chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '-' || value == '_')
    );
    assert!(
        challenge
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '-' || value == '_')
    );
}

#[test]
fn an_entropy_failure_stops_the_pair() {
    let entropy = TestEntropy {
        bytes: [0; 32],
        fail: true,
    };
    assert!(matches!(
        pkce_pair(&entropy),
        Err(XaiLoginError::Entropy { .. })
    ));
}

#[test]
fn the_authorization_url_carries_the_pinned_query() {
    let url = build_authorize_url(ENDPOINT, REDIRECT, CHALLENGE, STATE, NONCE);
    assert_eq!(url, URL);
}

#[test]
fn os_entropy_produces_distinct_bytes() {
    let mut first = [0_u8; 32];
    let mut second = [0_u8; 32];
    OsEntropy.fill(&mut first).unwrap();
    OsEntropy.fill(&mut second).unwrap();
    assert_ne!(first, second);
}

#[test]
fn random_uuids_are_distinct_hex_ids() {
    let first = RandomUuids.next();
    let second = RandomUuids.next();
    assert_eq!(first.len(), 32);
    assert!(first.chars().all(|value| value.is_ascii_hexdigit()));
    assert_ne!(first, second);
}

#![cfg(test)]

use std::ops::Add as _;

use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use super::{PostForm, authorize_post};
use crate::byok_consent::ConsentError;
use crate::byok_consent::store::{AUTH_CODE_TTL_SECONDS, AUTH_CODES_MAX_SIZE, AuthCodeStore};
use crate::byok_consent::test_support::seeded_store;
use crate::byok_token_exchange::ports::AuthCodeLookup as _;

const NOW: f64 = 1_700_000_000.0;
const LIVE_UNTIL: f64 = 1_700_000_300.0;
const ALREADY_EXPIRED: f64 = 1_699_999_999.0;
const CALLBACK: &str = "http://127.0.0.1:1456/callback";

fn form() -> PostForm {
    PostForm {
        client_id: "client-1".to_owned(),
        redirect_uri: CALLBACK.to_owned(),
        code_challenge: "challenge".to_owned(),
        code_challenge_method: "S256".to_owned(),
        state: "state-1".to_owned(),
        server_id: "srv-1".to_owned(),
        api_key: SecretString::from("sk-user"),
    }
}

fn is_uuid4(code: &str) -> bool {
    Uuid::parse_str(code)
        .is_ok_and(|parsed| parsed.get_version_num() == 4 && parsed.to_string() == code)
}

#[test]
fn the_stored_record_holds_every_form_field() {
    let codes = AuthCodeStore::new();
    let accepted = authorize_post(&form(), "user-1", &codes, NOW).unwrap();
    let stored = codes.get(&accepted.code).unwrap();
    assert_eq!(stored.api_key.expose_secret(), "sk-user");
    assert_eq!(stored.server_id, "srv-1");
    assert_eq!(stored.code_challenge, "challenge");
    assert_eq!(stored.redirect_uri, CALLBACK);
    assert_eq!(stored.client_id, "client-1");
    assert_eq!(stored.user_id, "user-1");
    assert_eq!(
        stored.expires_at.to_bits(),
        NOW.add(f64::from(AUTH_CODE_TTL_SECONDS)).to_bits()
    );
    assert_eq!(codes.count(), 1);
}

#[test]
fn the_generated_code_is_a_uuid() {
    let codes = AuthCodeStore::new();
    let accepted = authorize_post(&form(), "user-1", &codes, NOW).unwrap();
    assert!(is_uuid4(&accepted.code));
}

#[test]
fn the_location_carries_the_code_and_the_state() {
    let codes = AuthCodeStore::new();
    let accepted = authorize_post(&form(), "user-1", &codes, NOW).unwrap();
    assert_eq!(
        accepted.location,
        format!("{CALLBACK}?code={}&state=state-1", accepted.code)
    );
}

#[test]
fn the_location_uses_an_ampersand_when_the_uri_already_has_a_query() {
    let codes = AuthCodeStore::new();
    let form = PostForm {
        redirect_uri: "http://127.0.0.1:1456/callback?tenant=acme".to_owned(),
        ..form()
    };
    let accepted = authorize_post(&form, "user-1", &codes, NOW).unwrap();
    assert_eq!(
        accepted.location,
        format!(
            "http://127.0.0.1:1456/callback?tenant=acme&code={}&state=state-1",
            accepted.code
        )
    );
}

#[test]
fn the_state_is_quoted_like_urlencode() {
    let codes = AuthCodeStore::new();
    let form = PostForm {
        state: "a b/c&d=e".to_owned(),
        ..form()
    };
    let accepted = authorize_post(&form, "user-1", &codes, NOW).unwrap();
    assert_eq!(
        accepted.location,
        format!("{CALLBACK}?code={}&state=a+b%2Fc%26d%3De", accepted.code)
    );
}

#[test]
fn the_challenge_method_must_be_s256() {
    let codes = AuthCodeStore::new();
    let form = PostForm {
        code_challenge_method: "plain".to_owned(),
        ..form()
    };
    let error = authorize_post(&form, "user-1", &codes, NOW).unwrap_err();
    assert_eq!(
        error.status(),
        ConsentError::UnsupportedChallengeMethod.status()
    );
    assert_eq!(
        error.body(),
        ConsentError::UnsupportedChallengeMethod.body()
    );
    assert_eq!(codes.count(), 0);
}

#[test]
fn an_empty_user_id_is_login_required() {
    let codes = AuthCodeStore::new();
    let error = authorize_post(&form(), "", &codes, NOW).unwrap_err();
    assert_eq!(error.body(), ConsentError::LoginRequired.body());
    assert_eq!(codes.count(), 0);
}

#[test]
fn a_bad_redirect_uri_is_refused_before_the_capacity_check() {
    let codes = seeded_store(AUTH_CODES_MAX_SIZE, LIVE_UNTIL);
    let form = PostForm {
        redirect_uri: "https://example.com/callback".to_owned(),
        ..form()
    };
    let error = authorize_post(&form, "user-1", &codes, NOW).unwrap_err();
    assert!(matches!(error, ConsentError::RedirectUri(_)));
    assert_eq!(codes.count(), AUTH_CODES_MAX_SIZE);
}

#[test]
fn a_bad_redirect_uri_is_refused_before_the_method_check() {
    let codes = AuthCodeStore::new();
    let form = PostForm {
        redirect_uri: "https://example.com/callback".to_owned(),
        code_challenge_method: "plain".to_owned(),
        ..form()
    };
    let error = authorize_post(&form, "user-1", &codes, NOW).unwrap_err();
    assert!(matches!(error, ConsentError::RedirectUri(_)));
}

#[test]
fn the_capacity_check_precedes_the_method_check() {
    let codes = seeded_store(AUTH_CODES_MAX_SIZE, LIVE_UNTIL);
    let form = PostForm {
        code_challenge_method: "plain".to_owned(),
        ..form()
    };
    let error = authorize_post(&form, "user-1", &codes, NOW).unwrap_err();
    assert!(matches!(error, ConsentError::TooManyPendingFlows));
}

#[test]
fn the_method_check_precedes_the_identity_check() {
    let codes = AuthCodeStore::new();
    let form = PostForm {
        code_challenge_method: "plain".to_owned(),
        ..form()
    };
    let error = authorize_post(&form, "", &codes, NOW).unwrap_err();
    assert!(matches!(error, ConsentError::UnsupportedChallengeMethod));
}

#[test]
fn an_expired_prefill_frees_capacity() {
    let codes = seeded_store(AUTH_CODES_MAX_SIZE, ALREADY_EXPIRED);
    let accepted = authorize_post(&form(), "user-1", &codes, NOW).unwrap();
    assert_eq!(codes.count(), 1);
    assert!(codes.get(&accepted.code).is_some());
}

#[test]
fn two_flows_keep_their_own_codes() {
    let codes = AuthCodeStore::new();
    let first = authorize_post(&form(), "user-1", &codes, NOW).unwrap();
    let second = authorize_post(&form(), "user-2", &codes, NOW).unwrap();
    assert_ne!(first.code, second.code);
    assert_eq!(codes.count(), 2);
    assert_eq!(codes.get(&first.code).unwrap().user_id, "user-1");
    assert_eq!(codes.get(&second.code).unwrap().user_id, "user-2");
}

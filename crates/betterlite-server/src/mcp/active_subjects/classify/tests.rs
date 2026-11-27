#![cfg(test)]

use betterlite_auth::session_token_codec::LITELLM_SESSION_TOKEN_PREFIX;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_secrets::secret_codec::bearer::encrypt_bearer_token;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, StoredCipher};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::tests::{
    CountingNonces, FakeStore, SALT, fixed_now, session_codec, subjects, user_row,
};
use super::super::{ActiveSubjects, IdentityLookup};

const HEADER: &str = "eyJhbGciOiJub25lIn0";
const ISS_CONFIGURED: &str = "eyJpc3MiOiJodHRwczovL2lzc3Vlci5leGFtcGxlIn0";
const ISS_UNKNOWN: &str = "eyJpc3MiOiJodHRwczovL290aGVyLmV4YW1wbGUifQ";
const ISS_NUMBER: &str = "eyJpc3MiOjQyfQ";
const ISS_EMPTY: &str = "eyJpc3MiOiIifQ";
const NO_ISS: &str = "e30";

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn auth_headers(token: &str) -> HeaderMap {
    headers(&[("authorization", token)])
}

fn jwt(payload: &str) -> String {
    format!("{HEADER}.{payload}.")
}

fn with_issuers(global: Option<&str>, issuers: &[&str]) -> ActiveSubjects<FakeStore> {
    let mut active = subjects(FakeStore::new());
    active.settings.global_issuer = global.map(str::to_owned);
    active.settings.issuers = issuers.iter().map(|issuer| (*issuer).to_owned()).collect();
    active
}

#[tokio::test]
async fn header_api_key_present_answers_true_even_when_empty() {
    let active = subjects(FakeStore::new());
    let map = headers(&[("x-litellm-api-key", "")]);
    assert!(active.uses_gateway_credential(&map).await);
}

#[tokio::test]
async fn header_authorization_presence_without_a_token_answers_true() {
    let active = subjects(FakeStore::new());
    assert!(active.uses_gateway_credential(&auth_headers("")).await);
}

#[tokio::test]
async fn header_without_credentials_answers_false() {
    let active = subjects(FakeStore::new());
    assert!(!active.uses_gateway_credential(&HeaderMap::new()).await);
}

#[tokio::test]
async fn header_sk_token_answers_true() {
    let active = subjects(FakeStore::new());
    assert!(
        active
            .uses_gateway_credential(&auth_headers("Bearer sk-test"))
            .await
    );
}

#[tokio::test]
async fn header_master_key_answers_true() {
    let mut active = subjects(FakeStore::new());
    active.settings.master_key = Some(SecretString::from("master-secret"));
    assert!(
        active
            .uses_gateway_credential(&auth_headers("Bearer master-secret"))
            .await
    );
}

#[tokio::test]
async fn header_master_key_mismatch_answers_false() {
    let mut active = subjects(FakeStore::new());
    active.settings.master_key = Some(SecretString::from("master-secret"));
    assert!(
        !active
            .uses_gateway_credential(&auth_headers("Bearer master-secreu"))
            .await
    );
}

#[tokio::test]
async fn header_non_ascii_master_key_compares_the_encoded_bytes() {
    let mut active = subjects(FakeStore::new());
    active.settings.master_key = Some(SecretString::from("maîtresse"));
    assert!(
        active
            .uses_gateway_credential(&auth_headers("Bearer maîtresse"))
            .await
    );
}

#[tokio::test]
async fn header_empty_master_key_answers_false() {
    let mut active = subjects(FakeStore::new());
    active.settings.master_key = Some(SecretString::from(""));
    assert!(
        !active
            .uses_gateway_credential(&auth_headers("Bearer opaque-token"))
            .await
    );
}

#[tokio::test]
async fn header_custom_auth_answers_true() {
    let mut active = subjects(FakeStore::new());
    active.settings.custom_auth = true;
    assert!(
        active
            .uses_gateway_credential(&auth_headers("opaque-token"))
            .await
    );
}

#[tokio::test]
async fn header_oidc_userinfo_answers_true() {
    let mut active = subjects(FakeStore::new());
    active.settings.oidc_userinfo_enabled = true;
    assert!(
        active
            .uses_gateway_credential(&auth_headers("opaque-token"))
            .await
    );
}

#[tokio::test]
async fn jwt_missing_issuer_answers_true() {
    let active = with_issuers(Some("https://global.example"), &[]);
    assert!(
        active
            .uses_gateway_credential(&auth_headers(&jwt(NO_ISS)))
            .await
    );
}

#[tokio::test]
async fn jwt_non_string_issuer_answers_true() {
    let active = with_issuers(Some("https://global.example"), &[]);
    assert!(
        active
            .uses_gateway_credential(&auth_headers(&jwt(ISS_NUMBER)))
            .await
    );
}

#[tokio::test]
async fn jwt_empty_issuer_answers_true() {
    let active = with_issuers(Some("https://global.example"), &[]);
    assert!(
        active
            .uses_gateway_credential(&auth_headers(&jwt(ISS_EMPTY)))
            .await
    );
}

#[tokio::test]
async fn jwt_unset_global_issuer_answers_true() {
    let active = with_issuers(None, &[]);
    assert!(
        active
            .uses_gateway_credential(&auth_headers(&jwt(ISS_UNKNOWN)))
            .await
    );
}

#[tokio::test]
async fn jwt_empty_global_issuer_answers_true() {
    let active = with_issuers(Some(""), &[]);
    assert!(
        active
            .uses_gateway_credential(&auth_headers(&jwt(ISS_UNKNOWN)))
            .await
    );
}

#[tokio::test]
async fn jwt_global_issuer_match_answers_true() {
    let active = with_issuers(Some("https://issuer.example"), &[]);
    assert!(
        active
            .uses_gateway_credential(&auth_headers(&jwt(ISS_CONFIGURED)))
            .await
    );
}

#[tokio::test]
async fn jwt_configured_issuer_match_answers_true() {
    let active = with_issuers(Some("https://global.example"), &["https://issuer.example"]);
    assert!(
        active
            .uses_gateway_credential(&auth_headers(&jwt(ISS_CONFIGURED)))
            .await
    );
}

#[tokio::test]
async fn jwt_unknown_issuer_answers_false() {
    let active = with_issuers(
        Some("https://global.example"),
        &["https://configured.example"],
    );
    assert!(
        !active
            .uses_gateway_credential(&auth_headers(&jwt(ISS_UNKNOWN)))
            .await
    );
}

#[tokio::test]
async fn opaque_unknown_token_answers_false() {
    let active = subjects(FakeStore::new());
    assert!(
        !active
            .uses_gateway_credential(&auth_headers("opaque-token"))
            .await
    );
}

#[tokio::test]
async fn opaque_known_key_answers_true_and_hashes_the_token() {
    let store = FakeStore {
        identity: IdentityLookup::Found,
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert!(
        active
            .uses_gateway_credential(&auth_headers("opaque-token"))
            .await
    );
    assert_eq!(
        calls.lock().unwrap().identities,
        vec![hash_token("opaque-token")]
    );
}

#[tokio::test]
async fn opaque_faulted_lookup_answers_true() {
    let store = FakeStore {
        identity: IdentityLookup::Faulted,
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert!(
        active
            .uses_gateway_credential(&auth_headers("opaque-token"))
            .await
    );
}

#[rstest]
#[case::envelope("llm_env_sealed")]
#[case::refresh_envelope("llm_refresh_sealed")]
#[case::v2_gcm("v2:gcm:c2VhbGVk")]
#[tokio::test]
async fn opaque_prefixed_credentials_answer_true_without_a_lookup(#[case] token: &str) {
    let store = FakeStore::new();
    let calls = store.calls();
    let active = subjects(store);
    assert!(active.uses_gateway_credential(&auth_headers(token)).await);
    assert!(calls.lock().unwrap().identities.is_empty());
}

#[tokio::test]
async fn opaque_ui_session_token_answers_true() {
    let user = user_row(r#"{"user_id":"user-1","user_role":"internal_user","models":["gpt-4o"]}"#);
    let token = session_codec()
        .ui_session_token(&user, fixed_now())
        .unwrap();
    let active = subjects(FakeStore::new());
    assert!(
        active
            .uses_gateway_credential(&auth_headers(token.expose_secret()))
            .await
    );
}

#[tokio::test]
async fn opaque_unreadable_session_token_answers_true() {
    let cipher = StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT)),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: CountingNonces,
        diagnostics: TracingDiagnostics,
    };
    let token = encrypt_bearer_token(
        &cipher,
        &SecretString::from("not-json"),
        LITELLM_SESSION_TOKEN_PREFIX,
    )
    .unwrap();
    let store = FakeStore::new();
    let calls = store.calls();
    let active = subjects(store);
    assert!(
        active
            .uses_gateway_credential(&auth_headers(token.expose_secret()))
            .await
    );
    assert!(calls.lock().unwrap().identities.is_empty());
}

#![cfg(test)]

use betterlite_config::sections::mcp_bearer_credentials::{
    ResolvedPreviousKey, ResolvedSigning, Settings as SigningConfig,
};
use chrono::{DateTime, Utc};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::bridge_credentials::{
    ENCRYPTION_KEY_DOMAIN as BRIDGE_ENCRYPTION_KEY_DOMAIN,
    SIGNING_KEY_DOMAIN as BRIDGE_SIGNING_KEY_DOMAIN, scrypt_hex as bridge_scrypt_hex,
};
use crate::session_credentials::{
    SessionBearerResult, SessionRefreshResult, is_session_bearer_shaped,
    open_session_refresh_bearer, resolve_session_bearer, resolve_session_signing_keys,
    session_keys_from_master_key,
};
use crate::session_tokens::mint_session_token;
use crate::types::{SessionPrincipal, SessionSigningKeys, SessionTokenKind};

const PRIVATE: &str = include_str!("../../fixtures/session-private.pem");
const PREVIOUS_PUBLIC: &str = include_str!("../../fixtures/session-previous-public.pem");
const WEAK: &str = include_str!("../../fixtures/session-weak.pem");
const ENTROPY: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).expect("valid timestamp")
}

fn principal(client_id: &str) -> SessionPrincipal {
    SessionPrincipal {
        user_id: String::from("user-1"),
        client_id: client_id.to_owned(),
        resource_server_id: None,
        audience: None,
        team_id: None,
    }
}

fn hs256_keys() -> SessionSigningKeys {
    SessionSigningKeys::Symmetric(
        session_keys_from_master_key(&SecretString::from("a-sufficiently-long-master-key"))
            .expect("derive"),
    )
}

fn mint(keys: &SessionSigningKeys, kind: SessionTokenKind, client_id: &str) -> SecretString {
    mint_session_token(&principal(client_id), kind, keys, now(), || Ok(ENTROPY))
        .expect("mint")
        .token
}

#[test]
fn session_signing_key_derivation_is_deterministic() {
    let master_key = SecretString::from("a-sufficiently-long-master-key");
    let first = session_keys_from_master_key(&master_key).expect("derive");
    let second = session_keys_from_master_key(&master_key).expect("derive again");
    assert_eq!(
        first.signing_key.expose_secret(),
        second.signing_key.expose_secret()
    );
}

#[test]
fn session_signing_key_derivation_differs_across_master_keys() {
    let one = session_keys_from_master_key(&SecretString::from("master-key-one-long-enough"))
        .expect("derive");
    let two = session_keys_from_master_key(&SecretString::from("master-key-two-long-enough"))
        .expect("derive");
    assert_ne!(
        one.signing_key.expose_secret(),
        two.signing_key.expose_secret()
    );
}

#[test]
fn session_signing_key_domain_is_isolated_from_the_bridge_envelope_domains() {
    let master_key = SecretString::from("a-sufficiently-long-master-key");
    let session_signing = session_keys_from_master_key(&master_key).expect("derive");
    let bridge_signing =
        bridge_scrypt_hex(master_key.expose_secret(), BRIDGE_SIGNING_KEY_DOMAIN).expect("derive");
    let bridge_encryption =
        bridge_scrypt_hex(master_key.expose_secret(), BRIDGE_ENCRYPTION_KEY_DOMAIN)
            .expect("derive");
    assert_ne!(session_signing.signing_key.expose_secret(), bridge_signing);
    assert_ne!(
        session_signing.signing_key.expose_secret(),
        bridge_encryption
    );
}

#[rstest]
#[case::session_access("llm_session_whatever", true)]
#[case::session_refresh("llm_srefresh_whatever", true)]
#[case::bridge_envelope_is_not_session_shaped("llm_env_whatever", false)]
#[case::opaque_bearer_is_not_session_shaped("opaque-upstream-token", false)]
fn is_session_bearer_shaped_is_a_prefix_only_check(#[case] candidate: &str, #[case] shaped: bool) {
    assert_eq!(is_session_bearer_shaped(candidate), shaped);
    let with_bearer_scheme = format!("Bearer {candidate}");
    assert_eq!(is_session_bearer_shaped(&with_bearer_scheme), shaped);
    let with_mixed_case_scheme = format!("BEARER {candidate}");
    assert_eq!(is_session_bearer_shaped(&with_mixed_case_scheme), shaped);
}

#[test]
fn resolve_session_bearer_admits_a_valid_session_access_token() {
    let keys = hs256_keys();
    let token = mint(&keys, SessionTokenKind::Session, "client-1");
    let authorization = format!("Bearer {}", token.expose_secret());
    match resolve_session_bearer(&authorization, &keys, now()) {
        SessionBearerResult::SessionBearerAdmitted { principal, jti } => {
            assert_eq!(principal.client_id, "client-1");
            assert!(!jti.is_empty());
        }
        other @ (SessionBearerResult::NotSessionBearer
        | SessionBearerResult::SessionBearerInvalid { .. }) => {
            panic!("expected SessionBearerAdmitted, got {other:?}")
        }
    }
}

#[test]
fn resolve_session_bearer_is_not_a_session_bearer_for_a_plain_bearer() {
    let keys = hs256_keys();
    match resolve_session_bearer("Bearer opaque-upstream-token", &keys, now()) {
        SessionBearerResult::NotSessionBearer => {}
        other @ (SessionBearerResult::SessionBearerAdmitted { .. }
        | SessionBearerResult::SessionBearerInvalid { .. }) => {
            panic!("expected NotSessionBearer, got {other:?}")
        }
    }
}

#[test]
fn resolve_session_bearer_marks_an_expired_token_invalid_with_expired_true() {
    let keys = hs256_keys();
    let token = mint(&keys, SessionTokenKind::Session, "client-1");
    let far_future = now() + chrono::Duration::seconds(3_600);
    match resolve_session_bearer(token.expose_secret(), &keys, far_future) {
        SessionBearerResult::SessionBearerInvalid { expired } => assert!(expired),
        other @ (SessionBearerResult::NotSessionBearer
        | SessionBearerResult::SessionBearerAdmitted { .. }) => {
            panic!("expected SessionBearerInvalid, got {other:?}")
        }
    }
}

#[test]
fn resolve_session_bearer_marks_a_bad_signature_invalid_with_expired_false() {
    let keys = hs256_keys();
    let other_keys = SessionSigningKeys::Symmetric(
        session_keys_from_master_key(&SecretString::from("a-different-master-key-long-enough"))
            .expect("derive"),
    );
    let token = mint(&other_keys, SessionTokenKind::Session, "client-1");
    match resolve_session_bearer(token.expose_secret(), &keys, now()) {
        SessionBearerResult::SessionBearerInvalid { expired } => assert!(!expired),
        other @ (SessionBearerResult::NotSessionBearer
        | SessionBearerResult::SessionBearerAdmitted { .. }) => {
            panic!("expected SessionBearerInvalid, got {other:?}")
        }
    }
}

#[test]
fn resolve_session_bearer_rejects_a_refresh_token_presented_as_access() {
    let keys = hs256_keys();
    let token = mint(&keys, SessionTokenKind::Refresh, "client-1");
    match resolve_session_bearer(token.expose_secret(), &keys, now()) {
        SessionBearerResult::SessionBearerInvalid { expired } => assert!(!expired),
        other @ (SessionBearerResult::NotSessionBearer
        | SessionBearerResult::SessionBearerAdmitted { .. }) => {
            panic!("expected SessionBearerInvalid, got {other:?}")
        }
    }
}

#[test]
fn open_session_refresh_bearer_opens_a_valid_refresh_token() {
    let keys = hs256_keys();
    let token = mint(&keys, SessionTokenKind::Refresh, "client-1");
    match open_session_refresh_bearer(token.expose_secret(), &keys, now(), "client-1") {
        SessionRefreshResult::SessionRefreshOpened { principal, jti } => {
            assert_eq!(principal.client_id, "client-1");
            assert!(!jti.is_empty());
        }
        other @ SessionRefreshResult::SessionRefreshInvalid => {
            panic!("expected SessionRefreshOpened, got {other:?}")
        }
    }
}

#[test]
fn open_session_refresh_bearer_rejects_a_non_refresh_shaped_value() {
    let keys = hs256_keys();
    let token = mint(&keys, SessionTokenKind::Session, "client-1");
    match open_session_refresh_bearer(token.expose_secret(), &keys, now(), "client-1") {
        SessionRefreshResult::SessionRefreshInvalid => {}
        other @ SessionRefreshResult::SessionRefreshOpened { .. } => {
            panic!("expected SessionRefreshInvalid, got {other:?}")
        }
    }
}

#[test]
fn open_session_refresh_bearer_rejects_a_mismatched_client_id() {
    let keys = hs256_keys();
    let token = mint(&keys, SessionTokenKind::Refresh, "client-1");
    match open_session_refresh_bearer(token.expose_secret(), &keys, now(), "client-2") {
        SessionRefreshResult::SessionRefreshInvalid => {}
        other @ SessionRefreshResult::SessionRefreshOpened { .. } => {
            panic!("expected SessionRefreshInvalid, got {other:?}")
        }
    }
}

#[test]
fn resolve_session_signing_keys_defaults_to_hs256_when_settings_absent() {
    let master_key = SecretString::from("a-sufficiently-long-master-key");
    let keys = resolve_session_signing_keys(&master_key, &SigningConfig::Absent)
        .expect("resolves to HS256");
    match keys {
        SessionSigningKeys::Symmetric(symmetric) => {
            let expected = session_keys_from_master_key(&master_key).expect("derive");
            assert_eq!(
                symmetric.signing_key.expose_secret(),
                expected.signing_key.expose_secret()
            );
        }
        SessionSigningKeys::Asymmetric(_) => panic!("expected Symmetric"),
    }
}

#[test]
fn resolve_session_signing_keys_passes_through_an_invalid_configuration_as_an_error() {
    let master_key = SecretString::from("a-sufficiently-long-master-key");
    let settings = SigningConfig::Invalid(String::from("mcp_session_token_signing is malformed"));
    let error = resolve_session_signing_keys(&master_key, &settings)
        .expect_err("expected a SessionSigningConfigError");
    assert_eq!(error.detail, "mcp_session_token_signing is malformed");
}

#[test]
fn resolve_session_signing_keys_loads_a_valid_rs256_configuration() {
    let master_key = SecretString::from("a-sufficiently-long-master-key");
    let settings = SigningConfig::Resolved(ResolvedSigning {
        kid: String::from("current"),
        private_key_pem: SecretString::from(PRIVATE),
        previous_public_keys: vec![ResolvedPreviousKey {
            kid: String::from("previous"),
            public_key_pem: SecretString::from(PREVIOUS_PUBLIC),
        }],
    });
    let keys =
        resolve_session_signing_keys(&master_key, &settings).expect("resolves to Asymmetric");
    match keys {
        SessionSigningKeys::Asymmetric(asymmetric) => assert_eq!(asymmetric.kid, "current"),
        SessionSigningKeys::Symmetric(_) => panic!("expected Asymmetric"),
    }
}

#[test]
fn resolve_session_signing_keys_fails_closed_on_a_key_too_small_to_be_a_valid_rsa_key() {
    let master_key = SecretString::from("a-sufficiently-long-master-key");
    let settings = SigningConfig::Resolved(ResolvedSigning {
        kid: String::from("current"),
        private_key_pem: SecretString::from(WEAK),
        previous_public_keys: Vec::new(),
    });
    resolve_session_signing_keys(&master_key, &settings)
        .expect_err("a too-small RSA key must fail closed, never fall back to HS256");
}

#[test]
fn resolve_session_signing_keys_fails_closed_on_a_non_pem_private_key() {
    let master_key = SecretString::from("a-sufficiently-long-master-key");
    let settings = SigningConfig::Resolved(ResolvedSigning {
        kid: String::from("current"),
        private_key_pem: SecretString::from("not-a-pem-document"),
        previous_public_keys: Vec::new(),
    });
    resolve_session_signing_keys(&master_key, &settings)
        .expect_err("a non-PEM private key must fail closed, never fall back to HS256");
}

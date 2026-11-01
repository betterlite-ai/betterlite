#![cfg(test)]

use chrono::{DateTime, Utc};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use betterlite_secrets::secret_codec::OsNonces;

use crate::bridge_credentials::{
    BridgeEnvelopeResult, BridgeRefreshResult, ENCRYPTION_KEY_DOMAIN, SIGNING_KEY_DOMAIN,
    envelope_keys_from_master_key, is_bridge_envelope_shaped, open_bridge_refresh_envelope,
    resolve_bridge_envelope, scrypt_hex,
};
use crate::envelope::models::SubjectType;
use crate::envelope::{
    EnvelopeIdentity, EnvelopeKeys, RefreshCredential, UpstreamTokenGrant, mint_envelope,
    mint_refresh_envelope, open_envelope,
};

fn identity(server_id: &str) -> EnvelopeIdentity {
    EnvelopeIdentity {
        server_id: server_id.to_owned(),
        subject_type: SubjectType::KeyHash,
        subject: String::from("hash"),
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
}

#[test]
fn scrypt_hex_is_deterministic_for_the_same_master_key_and_domain() {
    let first = scrypt_hex("a-sufficiently-long-master-key", SIGNING_KEY_DOMAIN).expect("derive");
    let second =
        scrypt_hex("a-sufficiently-long-master-key", SIGNING_KEY_DOMAIN).expect("derive again");
    assert_eq!(first, second);
    assert_eq!(first.len(), 64);
}

#[test]
fn scrypt_hex_separates_signing_and_encryption_keys_by_domain() {
    let signing = scrypt_hex("a-sufficiently-long-master-key", SIGNING_KEY_DOMAIN).expect("derive");
    let encryption =
        scrypt_hex("a-sufficiently-long-master-key", ENCRYPTION_KEY_DOMAIN).expect("derive");
    assert_ne!(signing, encryption);
}

#[test]
fn scrypt_hex_differs_across_master_keys() {
    let one = scrypt_hex("master-key-one-long-enough", SIGNING_KEY_DOMAIN).expect("derive");
    let two = scrypt_hex("master-key-two-long-enough", SIGNING_KEY_DOMAIN).expect("derive");
    assert_ne!(one, two);
}

#[test]
fn envelope_keys_from_master_key_derives_working_keys_deterministically() {
    let master_key = SecretString::from("a-sufficiently-long-master-key");
    let first = envelope_keys_from_master_key(&master_key).expect("derive");
    let second = envelope_keys_from_master_key(&master_key).expect("derive again");
    let current = now();
    let sealed = mint_envelope(
        identity("server-a"),
        UpstreamTokenGrant {
            access_token: SecretString::from("access"),
            token_type: String::from("Bearer"),
            refresh_token: None,
            scope: None,
            expires_in: Some(60),
        },
        &first,
        current,
        &OsNonces,
    )
    .expect("mint with the first derivation");
    let opened = open_envelope(sealed.token.expose_secret(), &second, current)
        .expect("opens with a separately derived, but identical, key");
    assert_eq!(opened.identity.server_id, "server-a");
}

#[rstest]
#[case::bridge_envelope_access("llm_env_whatever", true)]
#[case::bridge_envelope_refresh("llm_refresh_whatever", true)]
#[case::session_bearer_is_not_bridge_shaped("llm_session_whatever", false)]
#[case::opaque_bearer_is_not_bridge_shaped("opaque-upstream-token", false)]
fn is_bridge_envelope_shaped_is_a_prefix_only_check(#[case] candidate: &str, #[case] shaped: bool) {
    assert_eq!(is_bridge_envelope_shaped(candidate), shaped);
    let with_bearer_scheme = format!("Bearer {candidate}");
    assert_eq!(is_bridge_envelope_shaped(&with_bearer_scheme), shaped);
    let with_mixed_case_scheme = format!("BEARER {candidate}");
    assert_eq!(is_bridge_envelope_shaped(&with_mixed_case_scheme), shaped);
}

fn envelope_keys() -> EnvelopeKeys {
    EnvelopeKeys::new(
        SecretString::from("signing-key-that-is-at-least-32-bytes-long"),
        SecretString::from("encryption-key"),
    )
    .expect("valid keys")
}

fn minted_access(keys: &EnvelopeKeys, server_id: &str, expires_in: i64) -> SecretString {
    mint_envelope(
        identity(server_id),
        UpstreamTokenGrant {
            access_token: SecretString::from("upstream-access-token"),
            token_type: String::from("bearer"),
            refresh_token: None,
            scope: None,
            expires_in: Some(expires_in),
        },
        keys,
        now(),
        &OsNonces,
    )
    .expect("mint")
    .token
}

fn minted_refresh(keys: &EnvelopeKeys, server_id: &str, expires_in: i64) -> SecretString {
    mint_refresh_envelope(
        identity(server_id),
        RefreshCredential {
            refresh_token: SecretString::from("upstream-refresh-token"),
            scope: None,
            expires_in: Some(expires_in),
        },
        keys,
        now(),
        &OsNonces,
    )
    .expect("mint")
    .token
}

#[test]
fn resolve_bridge_envelope_admits_a_valid_envelope_and_normalizes_the_scheme() {
    let keys = envelope_keys();
    let token = minted_access(&keys, "server-a", 60);
    let authorization = format!("Bearer {}", token.expose_secret());
    match resolve_bridge_envelope(&authorization, &keys, now(), "server-a") {
        BridgeEnvelopeResult::BridgeEnvelopeAdmitted {
            identity,
            upstream_authorization,
        } => {
            assert_eq!(identity.server_id, "server-a");
            assert_eq!(
                upstream_authorization.expose_secret(),
                "Bearer upstream-access-token"
            );
        }
        other @ (BridgeEnvelopeResult::NotBridgeEnvelope
        | BridgeEnvelopeResult::BridgeEnvelopeInvalid) => {
            panic!("expected BridgeEnvelopeAdmitted, got {other:?}")
        }
    }
}

#[test]
fn resolve_bridge_envelope_is_not_bridge_envelope_for_a_plain_bearer() {
    let keys = envelope_keys();
    match resolve_bridge_envelope("Bearer opaque-upstream-token", &keys, now(), "server-a") {
        BridgeEnvelopeResult::NotBridgeEnvelope => {}
        other @ (BridgeEnvelopeResult::BridgeEnvelopeAdmitted { .. }
        | BridgeEnvelopeResult::BridgeEnvelopeInvalid) => {
            panic!("expected NotBridgeEnvelope, got {other:?}")
        }
    }
}

#[test]
fn resolve_bridge_envelope_is_invalid_once_expired() {
    let keys = envelope_keys();
    let token = minted_access(&keys, "server-a", 60);
    let far_future = now() + chrono::Duration::seconds(3_600);
    match resolve_bridge_envelope(token.expose_secret(), &keys, far_future, "server-a") {
        BridgeEnvelopeResult::BridgeEnvelopeInvalid => {}
        other @ (BridgeEnvelopeResult::NotBridgeEnvelope
        | BridgeEnvelopeResult::BridgeEnvelopeAdmitted { .. }) => {
            panic!("expected BridgeEnvelopeInvalid, got {other:?}")
        }
    }
}

#[test]
fn resolve_bridge_envelope_is_invalid_for_a_bad_signature() {
    let keys = envelope_keys();
    let other_keys = EnvelopeKeys::new(
        SecretString::from("a-different-signing-key-at-least-32-bytes"),
        SecretString::from("a-different-encryption-key"),
    )
    .expect("valid keys");
    let token = minted_access(&other_keys, "server-a", 60);
    match resolve_bridge_envelope(token.expose_secret(), &keys, now(), "server-a") {
        BridgeEnvelopeResult::BridgeEnvelopeInvalid => {}
        other @ (BridgeEnvelopeResult::NotBridgeEnvelope
        | BridgeEnvelopeResult::BridgeEnvelopeAdmitted { .. }) => {
            panic!("expected BridgeEnvelopeInvalid, got {other:?}")
        }
    }
}

#[test]
fn resolve_bridge_envelope_rejects_a_mismatched_server_id() {
    let keys = envelope_keys();
    let token = minted_access(&keys, "server-a", 60);
    match resolve_bridge_envelope(token.expose_secret(), &keys, now(), "server-b") {
        BridgeEnvelopeResult::BridgeEnvelopeInvalid => {}
        other @ (BridgeEnvelopeResult::NotBridgeEnvelope
        | BridgeEnvelopeResult::BridgeEnvelopeAdmitted { .. }) => {
            panic!("expected BridgeEnvelopeInvalid, got {other:?}")
        }
    }
}

#[test]
fn resolve_bridge_envelope_rejects_a_refresh_envelope_presented_as_access() {
    let keys = envelope_keys();
    let token = minted_refresh(&keys, "server-a", 60);
    match resolve_bridge_envelope(token.expose_secret(), &keys, now(), "server-a") {
        BridgeEnvelopeResult::BridgeEnvelopeInvalid => {}
        other @ (BridgeEnvelopeResult::NotBridgeEnvelope
        | BridgeEnvelopeResult::BridgeEnvelopeAdmitted { .. }) => {
            panic!("expected BridgeEnvelopeInvalid, got {other:?}")
        }
    }
}

#[test]
fn open_bridge_refresh_envelope_opens_a_valid_refresh_envelope() {
    let keys = envelope_keys();
    let token = minted_refresh(&keys, "server-a", 1_000);
    match open_bridge_refresh_envelope(token.expose_secret(), &keys, now(), "server-a") {
        BridgeRefreshResult::BridgeRefreshOpened { identity, refresh } => {
            assert_eq!(identity.server_id, "server-a");
            assert_eq!(
                refresh.refresh_token.expose_secret(),
                "upstream-refresh-token"
            );
        }
        other @ BridgeRefreshResult::BridgeRefreshInvalid => {
            panic!("expected BridgeRefreshOpened, got {other:?}")
        }
    }
}

#[test]
fn open_bridge_refresh_envelope_rejects_a_non_refresh_shaped_value() {
    let keys = envelope_keys();
    let token = minted_access(&keys, "server-a", 60);
    match open_bridge_refresh_envelope(token.expose_secret(), &keys, now(), "server-a") {
        BridgeRefreshResult::BridgeRefreshInvalid => {}
        other @ BridgeRefreshResult::BridgeRefreshOpened { .. } => {
            panic!("expected BridgeRefreshInvalid, got {other:?}")
        }
    }
}

#[test]
fn open_bridge_refresh_envelope_rejects_a_mismatched_server_id() {
    let keys = envelope_keys();
    let token = minted_refresh(&keys, "server-a", 1_000);
    match open_bridge_refresh_envelope(token.expose_secret(), &keys, now(), "server-b") {
        BridgeRefreshResult::BridgeRefreshInvalid => {}
        other @ BridgeRefreshResult::BridgeRefreshOpened { .. } => {
            panic!("expected BridgeRefreshInvalid, got {other:?}")
        }
    }
}

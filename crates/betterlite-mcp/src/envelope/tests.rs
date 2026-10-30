#![cfg(test)]

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    EnvelopeIdentity, EnvelopeKeys, EnvelopeOpenError, RefreshCredential, UpstreamTokenGrant,
    is_envelope, is_refresh_envelope, mint_envelope, mint_refresh_envelope, open_envelope,
    open_refresh_envelope,
};

fn keys() -> EnvelopeKeys {
    EnvelopeKeys {
        signing_key: SecretString::from("signing-key-that-is-at-least-32-bytes-long"),
        encryption_key: SecretString::from("encryption-key"),
    }
}

fn identity() -> EnvelopeIdentity {
    EnvelopeIdentity {
        server_id: String::from("server"),
        subject_type: String::from("key_hash"),
        subject: String::from("hash"),
    }
}

fn grant() -> UpstreamTokenGrant {
    UpstreamTokenGrant {
        access_token: SecretString::from("access-secret"),
        token_type: String::from("Bearer"),
        refresh_token: Some(SecretString::from("refresh-secret")),
        scope: Some(String::from("read")),
        expires_in: Some(60),
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
}

#[test]
fn access_round_trip_keeps_tokens_secret() {
    let current = now();
    let signing_keys = keys();
    let sealed = mint_envelope(identity(), grant(), &signing_keys, current).expect("mint");
    assert!(is_envelope(sealed.token.expose_secret()));
    assert!(!sealed.token.expose_secret().contains("access-secret"));
    let opened = open_envelope(sealed.token.expose_secret(), &signing_keys, current).expect("open");
    assert_eq!(opened.identity, identity());
    assert_eq!(
        opened.grant.access_token.expose_secret(),
        grant().access_token.expose_secret()
    );
    assert!(!format!("{opened:?}").contains("access-secret"));
}

#[test]
fn refresh_round_trip_and_kind_isolation() {
    let current = now();
    let refresh = RefreshCredential {
        refresh_token: SecretString::from("refresh-secret"),
        scope: Some(String::from("read")),
        expires_in: Some(1_500_000),
    };
    let signing_keys = keys();
    let sealed =
        mint_refresh_envelope(identity(), refresh.clone(), &signing_keys, current).expect("mint");
    assert!(is_refresh_envelope(sealed.token.expose_secret()));
    assert!(!is_envelope(sealed.token.expose_secret()));
    let opened =
        open_refresh_envelope(sealed.token.expose_secret(), &signing_keys, current).expect("open");
    assert_eq!(
        opened.refresh.refresh_token.expose_secret(),
        refresh.refresh_token.expose_secret()
    );
    assert!(matches!(
        open_envelope(sealed.token.expose_secret(), &signing_keys, current),
        Err(EnvelopeOpenError::NotAnEnvelope)
    ));
}

#[test]
fn wrong_signature_and_expiry_are_discriminated() {
    let current = now();
    let signing_keys = keys();
    let sealed = mint_envelope(identity(), grant(), &signing_keys, current).expect("mint");
    let wrong = EnvelopeKeys {
        signing_key: SecretString::from("different-signing-key-that-is-32-bytes"),
        encryption_key: SecretString::from("encryption-key"),
    };
    assert!(matches!(
        open_envelope(sealed.token.expose_secret(), &wrong, current),
        Err(EnvelopeOpenError::BadSignature)
    ));
    assert!(matches!(
        open_envelope(
            sealed.token.expose_secret(),
            &signing_keys,
            current + chrono::TimeDelta::seconds(61),
        ),
        Err(EnvelopeOpenError::Expired)
    ));
}

#[test]
fn malformed_and_decryption_failures_do_not_panic() {
    assert!(matches!(
        open_envelope("llm_env_not-a-jwt", &keys(), now()),
        Err(EnvelopeOpenError::MalformedPayload)
    ));
    let current = now();
    let signing_keys = keys();
    let sealed = mint_envelope(identity(), grant(), &signing_keys, current).expect("mint");
    let wrong = EnvelopeKeys {
        signing_key: signing_keys.signing_key,
        encryption_key: SecretString::from("other-encryption-key"),
    };
    assert!(matches!(
        open_envelope(sealed.token.expose_secret(), &wrong, current),
        Err(EnvelopeOpenError::DecryptFailed)
    ));
}

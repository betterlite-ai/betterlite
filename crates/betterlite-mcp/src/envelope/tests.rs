#![cfg(test)]

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::{NonceSource, OsNonces};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    EnvelopeIdentity, EnvelopeKeys, EnvelopeOpenError, RefreshCredential, UpstreamTokenGrant,
    is_envelope, is_refresh_envelope, mint_envelope, mint_refresh_envelope, open_envelope,
    open_refresh_envelope,
};
use super::{
    crypto,
    models::{Claims, EnvelopeKind, SubjectType},
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
        subject_type: SubjectType::KeyHash,
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

struct FixedNonce;

impl NonceSource for FixedNonce {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        for (index, byte) in nonce.iter_mut().enumerate() {
            let Ok(value) = u8::try_from(index) else {
                return Err(SecretError::Truncated);
            };
            *byte = value;
        }
        Ok(())
    }
}

#[test]
fn access_round_trip_keeps_tokens_secret() {
    let current = now();
    let signing_keys = keys();
    let sealed =
        mint_envelope(identity(), grant(), &signing_keys, current, &OsNonces).expect("mint");
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
    let sealed = mint_refresh_envelope(
        identity(),
        refresh.clone(),
        &signing_keys,
        current,
        &OsNonces,
    )
    .expect("mint");
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
    let sealed =
        mint_envelope(identity(), grant(), &signing_keys, current, &OsNonces).expect("mint");
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
    let sealed =
        mint_envelope(identity(), grant(), &signing_keys, current, &OsNonces).expect("mint");
    let wrong = EnvelopeKeys {
        signing_key: signing_keys.signing_key,
        encryption_key: SecretString::from("other-encryption-key"),
    };
    assert!(matches!(
        open_envelope(sealed.token.expose_secret(), &wrong, current),
        Err(EnvelopeOpenError::DecryptFailed)
    ));
}

#[test]
fn ttl_selection_is_reflected_in_sealed_expiry() {
    let current = now();
    let signing_keys = keys();
    let access =
        mint_envelope(identity(), grant(), &signing_keys, current, &FixedNonce).expect("mint");
    assert_eq!(access.expires_at, current + chrono::TimeDelta::seconds(60));
    let mut no_expiry = grant();
    no_expiry.expires_in = None;
    let fallback =
        mint_envelope(identity(), no_expiry, &signing_keys, current, &FixedNonce).expect("mint");
    assert_eq!(fallback.expires_at, current + chrono::TimeDelta::hours(1));
    let refresh = RefreshCredential {
        refresh_token: SecretString::from("refresh-secret"),
        scope: None,
        expires_in: Some(1_500_000),
    };
    let capped = mint_refresh_envelope(identity(), refresh, &signing_keys, current, &FixedNonce)
        .expect("mint");
    assert_eq!(
        capped.expires_at,
        current + chrono::TimeDelta::seconds(super::MAX_REFRESH_TTL_SECONDS)
    );
    let uncapped = RefreshCredential {
        refresh_token: SecretString::from("refresh-secret"),
        scope: None,
        expires_in: None,
    };
    let refresh_fallback =
        mint_refresh_envelope(identity(), uncapped, &signing_keys, current, &FixedNonce)
            .expect("mint");
    assert_eq!(refresh_fallback.expires_at, capped.expires_at);
}

#[test]
fn mint_rejects_key_and_identity_validation() {
    let current = now();
    let short_keys = EnvelopeKeys {
        signing_key: SecretString::from("short"),
        encryption_key: SecretString::from("encryption-key"),
    };
    assert!(matches!(
        mint_envelope(identity(), grant(), &short_keys, current, &FixedNonce),
        Err(super::models::EnvelopeMintError::SigningKeyTooShort)
    ));
    let empty_encryption = EnvelopeKeys {
        signing_key: keys().signing_key,
        encryption_key: SecretString::from(""),
    };
    assert!(matches!(
        mint_envelope(identity(), grant(), &empty_encryption, current, &FixedNonce),
        Err(super::models::EnvelopeMintError::EncryptionKeyEmpty)
    ));
    let mut invalid_identity = identity();
    invalid_identity.server_id.clear();
    assert!(matches!(
        mint_envelope(invalid_identity, grant(), &keys(), current, &FixedNonce),
        Err(super::models::EnvelopeMintError::IdentityFieldEmpty)
    ));
}

#[test]
fn mint_rejects_grant_validation_and_limits() {
    let current = now();
    let mut invalid_grant = grant();
    invalid_grant.access_token = SecretString::from("");
    assert!(matches!(
        mint_envelope(identity(), invalid_grant, &keys(), current, &FixedNonce),
        Err(super::models::EnvelopeMintError::TokenEmpty)
    ));
    let mut invalid_expiry = grant();
    invalid_expiry.expires_in = Some(0);
    assert!(matches!(
        mint_envelope(identity(), invalid_expiry, &keys(), current, &FixedNonce),
        Err(super::models::EnvelopeMintError::ExpiresInInvalid)
    ));
    let mut oversized = grant();
    oversized.access_token = SecretString::from("x".repeat(20_000));
    assert!(matches!(
        mint_envelope(identity(), oversized, &keys(), current, &FixedNonce),
        Err(super::models::EnvelopeMintError::EnvelopeTooLarge { .. })
    ));
    let mut unrepresentable = grant();
    unrepresentable.expires_in = Some(i64::MAX);
    assert!(matches!(
        mint_envelope(identity(), unrepresentable, &keys(), current, &FixedNonce),
        Err(super::models::EnvelopeMintError::EnvelopeUnrepresentable { .. })
    ));
}

#[test]
fn wrong_prefix_with_signed_wrong_kind_is_malformed() {
    let current = now();
    let sealed = mint_envelope(identity(), grant(), &keys(), current, &FixedNonce).expect("mint");
    let refresh_prefix = sealed.token.expose_secret().replacen(
        super::ENVELOPE_PREFIX,
        super::REFRESH_ENVELOPE_PREFIX,
        1,
    );
    assert!(matches!(
        open_refresh_envelope(&refresh_prefix, &keys(), current),
        Err(EnvelopeOpenError::MalformedPayload)
    ));
}

#[test]
fn open_rejects_a_decrypted_grant_that_breaks_model_constraints() {
    let current = now();
    let signing_keys = keys();
    let plaintext = r#"{"access_token":"","token_type":"Bearer","expires_in":0}"#;
    let grant =
        crypto::encrypt_grant(plaintext, &signing_keys.encryption_key, &FixedNonce).expect("seal");
    let claims = Claims {
        iss: super::ENVELOPE_ISSUER.to_owned(),
        iat: current.timestamp(),
        exp: (current + chrono::TimeDelta::hours(1)).timestamp(),
        kind: EnvelopeKind::Access,
        server_id: String::from("server"),
        subject_type: SubjectType::KeyHash,
        subject: String::from("hash"),
        grant,
    };
    let compact = crypto::encode_jwt(&claims, &signing_keys.signing_key).expect("sign");
    let candidate = format!("{}{}", super::ENVELOPE_PREFIX, compact);
    assert!(matches!(
        open_envelope(&candidate, &signing_keys, current),
        Err(EnvelopeOpenError::MalformedPayload)
    ));
}

#[test]
fn opens_pinned_litellm_access_fixture() {
    let fixture = "llm_env_eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJsaXRlbGxtLW1jcC1icmlkZ2UiLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6MTcwMDAwMzYwMCwia2luZCI6ImFjY2VzcyIsInNlcnZlcl9pZCI6InNlcnZlciIsInN1YmplY3RfdHlwZSI6ImtleV9oYXNoIiwic3ViamVjdCI6Imhhc2giLCJncmFudCI6IkFBRUNBd1FGQmdjSUNRb0xEQTBPRHhBUkVoTVVGUllYLTVpYWlpUUw1M3djVFhXazhkRlp6RzQyZTUzOE9ONU1xOHU3QXFGcmw2NWtYcUVOS3BUQldPcFFpMnhpdFB2UDR1ZGxObEZVUTFyM0t3SHRyQXFOVVVTcXVzQi1kem1hNFp6WVk1aW56MjBEVjI4aHpRRVkifQ.pomMRIu8Q_Hq6CCIfJOcDYkrogqgB84AsNLOdEcgKoQ";
    let opened = open_envelope(fixture, &keys(), now()).expect("open fixture");
    assert_eq!(opened.grant.access_token.expose_secret(), "fixture-access");
}

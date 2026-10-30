#![cfg(test)]

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::{NonceSource, OsNonces};
use chrono::{DateTime, Utc};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::{fs, path::PathBuf, str};

use super::{
    ENVELOPE_PREFIX, EnvelopeIdentity, EnvelopeKeys, EnvelopeOpenError, EnvelopePrefix as _,
    REFRESH_ENVELOPE_PREFIX, RefreshCredential, UpstreamTokenGrant, mint_envelope,
    mint_refresh_envelope, open_envelope, open_refresh_envelope,
};
use super::{
    crypto,
    models::{Claims, EnvelopeKind, EnvelopeMintError, SubjectType},
};

fn keys() -> EnvelopeKeys {
    EnvelopeKeys::new(
        SecretString::from("signing-key-that-is-at-least-32-bytes-long"),
        SecretString::from("encryption-key"),
    )
    .expect("valid keys")
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
    assert!(sealed.token.expose_secret().is_envelope());
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
    assert!(sealed.token.expose_secret().is_refresh_envelope());
    assert!(!sealed.token.expose_secret().is_envelope());
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
    let wrong = EnvelopeKeys::new(
        SecretString::from("different-signing-key-that-is-32-bytes"),
        SecretString::from("encryption-key"),
    )
    .expect("valid keys");
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
    let wrong = EnvelopeKeys::new(
        signing_keys.signing_key,
        SecretString::from("other-encryption-key"),
    )
    .expect("valid keys");
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
    assert!(matches!(
        EnvelopeKeys::new(
            SecretString::from("short"),
            SecretString::from("encryption-key")
        ),
        Err(EnvelopeMintError::SigningKeyTooShort)
    ));
    assert!(matches!(
        EnvelopeKeys::new(keys().signing_key, SecretString::from("")),
        Err(EnvelopeMintError::EncryptionKeyEmpty)
    ));
    let mut invalid_identity = identity();
    invalid_identity.server_id.clear();
    assert!(matches!(
        mint_envelope(invalid_identity, grant(), &keys(), current, &FixedNonce),
        Err(EnvelopeMintError::IdentityFieldEmpty)
    ));
}

#[test]
fn mint_rejects_grant_validation_and_limits() {
    let current = now();
    let mut invalid_grant = grant();
    invalid_grant.access_token = SecretString::from("");
    assert!(matches!(
        mint_envelope(identity(), invalid_grant, &keys(), current, &FixedNonce),
        Err(EnvelopeMintError::TokenEmpty)
    ));
    let mut invalid_expiry = grant();
    invalid_expiry.expires_in = Some(0);
    assert!(matches!(
        mint_envelope(identity(), invalid_expiry, &keys(), current, &FixedNonce),
        Err(EnvelopeMintError::ExpiresInInvalid)
    ));
    let mut oversized = grant();
    oversized.access_token = SecretString::from("x".repeat(20_000));
    assert!(matches!(
        mint_envelope(identity(), oversized, &keys(), current, &FixedNonce),
        Err(EnvelopeMintError::EnvelopeTooLarge { size_bytes, max_bytes })
            if size_bytes > super::MAX_ENVELOPE_BYTES && max_bytes == super::MAX_ENVELOPE_BYTES
    ));
    let mut unrepresentable = grant();
    unrepresentable.expires_in = Some(i64::MAX);
    assert!(matches!(
        mint_envelope(identity(), unrepresentable, &keys(), current, &FixedNonce),
        Err(EnvelopeMintError::EnvelopeUnrepresentable {
            expires_in: i64::MAX
        })
    ));
}

fn signed_candidate(kind: EnvelopeKind, plaintext: &str, prefix: &str) -> String {
    let signing_keys = keys();
    let grant =
        crypto::encrypt_grant(plaintext, &signing_keys.encryption_key, &FixedNonce).expect("seal");
    let claims = Claims {
        iss: super::ENVELOPE_ISSUER.to_owned(),
        iat: now().timestamp(),
        exp: now()
            .checked_add_signed(chrono::TimeDelta::hours(1))
            .unwrap()
            .timestamp(),
        kind,
        server_id: String::from("server"),
        subject_type: SubjectType::KeyHash,
        subject: String::from("hash"),
        grant,
    };
    let compact = crypto::encode_jwt(&claims, &signing_keys.signing_key).expect("sign");
    format!("{prefix}{compact}")
}

#[test]
fn wrong_prefix_with_signed_wrong_kind_is_malformed() {
    let plaintext = r#"{"refresh_token":"r"}"#;
    let valid = signed_candidate(EnvelopeKind::Refresh, plaintext, REFRESH_ENVELOPE_PREFIX);
    open_refresh_envelope(&valid, &keys(), now()).unwrap();
    let candidate = signed_candidate(EnvelopeKind::Access, plaintext, REFRESH_ENVELOPE_PREFIX);
    assert_eq!(
        open_refresh_envelope(&candidate, &keys(), now()).unwrap_err(),
        EnvelopeOpenError::MalformedPayload
    );
}

#[test]
fn refresh_kind_cannot_open_as_access() {
    let plaintext = r#"{"access_token":"a","token_type":"Bearer"}"#;
    let valid = signed_candidate(EnvelopeKind::Access, plaintext, ENVELOPE_PREFIX);
    open_envelope(&valid, &keys(), now()).unwrap();
    let candidate = signed_candidate(EnvelopeKind::Refresh, plaintext, ENVELOPE_PREFIX);
    assert_eq!(
        open_envelope(&candidate, &keys(), now()).unwrap_err(),
        EnvelopeOpenError::MalformedPayload
    );
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
    let candidate = format!("{ENVELOPE_PREFIX}{compact}");
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

#[rstest]
#[case::raw("", EnvelopeOpenError::NotAnEnvelope)]
#[case::access(ENVELOPE_PREFIX, EnvelopeOpenError::MalformedPayload)]
#[case::refresh(REFRESH_ENVELOPE_PREFIX, EnvelopeOpenError::NotAnEnvelope)]
fn oversized_access_candidates(#[case] prefix: &str, #[case] expected: EnvelopeOpenError) {
    let candidate = format!("{prefix}{}", "x".repeat(super::MAX_ENVELOPE_BYTES + 1));
    assert_eq!(
        open_envelope(&candidate, &keys(), now()).unwrap_err(),
        expected
    );
}

#[rstest]
#[case::raw("", EnvelopeOpenError::NotAnEnvelope)]
#[case::access(ENVELOPE_PREFIX, EnvelopeOpenError::NotAnEnvelope)]
#[case::refresh(REFRESH_ENVELOPE_PREFIX, EnvelopeOpenError::MalformedPayload)]
fn oversized_refresh_candidates(#[case] prefix: &str, #[case] expected: EnvelopeOpenError) {
    let candidate = format!("{prefix}{}", "x".repeat(super::MAX_ENVELOPE_BYTES + 1));
    assert_eq!(
        open_refresh_envelope(&candidate, &keys(), now()).unwrap_err(),
        expected
    );
}

#[rstest]
#[case::empty_server("", "subject")]
#[case::empty_subject("server", "")]
fn mint_rejects_empty_identity(#[case] server: &str, #[case] subject: &str) {
    let invalid = EnvelopeIdentity {
        server_id: server.to_owned(),
        subject_type: SubjectType::UserId,
        subject: subject.to_owned(),
    };
    let refresh = RefreshCredential {
        refresh_token: SecretString::from("r"),
        scope: None,
        expires_in: None,
    };
    assert!(matches!(
        mint_envelope(invalid.clone(), grant(), &keys(), now(), &FixedNonce),
        Err(EnvelopeMintError::IdentityFieldEmpty)
    ));
    assert!(matches!(
        mint_refresh_envelope(invalid, refresh, &keys(), now(), &FixedNonce),
        Err(EnvelopeMintError::IdentityFieldEmpty)
    ));
}

#[rstest]
#[case::empty_access("", "Bearer", Some("r"))]
#[case::empty_type("a", "", Some("r"))]
#[case::empty_refresh("a", "Bearer", Some(""))]
fn mint_rejects_empty_tokens(
    #[case] access: &str,
    #[case] token_type: &str,
    #[case] refresh: Option<&str>,
) {
    let invalid = UpstreamTokenGrant {
        access_token: SecretString::from(access),
        token_type: token_type.to_owned(),
        refresh_token: refresh.map(SecretString::from),
        scope: None,
        expires_in: None,
    };
    assert!(matches!(
        mint_envelope(identity(), invalid, &keys(), now(), &FixedNonce),
        Err(EnvelopeMintError::TokenEmpty)
    ));
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
fn mint_rejects_nonpositive_ttl(#[case] lifetime: i64) {
    let mut access = grant();
    access.expires_in = Some(lifetime);
    let refresh = RefreshCredential {
        refresh_token: SecretString::from("r"),
        scope: None,
        expires_in: Some(lifetime),
    };
    assert!(matches!(
        mint_envelope(identity(), access, &keys(), now(), &FixedNonce),
        Err(EnvelopeMintError::ExpiresInInvalid)
    ));
    assert!(matches!(
        mint_refresh_envelope(identity(), refresh, &keys(), now(), &FixedNonce),
        Err(EnvelopeMintError::ExpiresInInvalid)
    ));
}

#[rstest]
#[case::empty_token("", None, true)]
#[case::zero_ttl("r", Some(0), false)]
#[case::negative_ttl("r", Some(-1), false)]
fn refresh_mint_constraints(
    #[case] token: &str,
    #[case] expires_in: Option<i64>,
    #[case] token_error: bool,
) {
    let refresh = RefreshCredential {
        refresh_token: SecretString::from(token),
        scope: None,
        expires_in,
    };
    let error =
        mint_refresh_envelope(identity(), refresh, &keys(), now(), &FixedNonce).unwrap_err();
    if token_error {
        assert!(matches!(error, EnvelopeMintError::TokenEmpty));
    } else {
        assert!(matches!(error, EnvelopeMintError::ExpiresInInvalid));
    }
}

#[rstest]
#[case::empty_token(r#"{"refresh_token":""}"#)]
#[case::zero_ttl(r#"{"refresh_token":"r","expires_in":0}"#)]
#[case::negative_ttl(r#"{"refresh_token":"r","expires_in":-1}"#)]
fn open_refresh_constraints(#[case] plaintext: &str) {
    let candidate = signed_candidate(EnvelopeKind::Refresh, plaintext, REFRESH_ENVELOPE_PREFIX);
    assert_eq!(
        open_refresh_envelope(&candidate, &keys(), now()).unwrap_err(),
        EnvelopeOpenError::MalformedPayload
    );
}

#[rstest]
#[case::access(EnvelopeKind::Access)]
#[case::refresh(EnvelopeKind::Refresh)]
fn oversized_mint_reports_sizes(#[case] kind: EnvelopeKind) {
    let token = "x".repeat(20_000);
    let (error, plaintext, prefix) = match kind {
        EnvelopeKind::Access => {
            let mut oversized = grant();
            oversized.access_token = SecretString::from(token);
            let plaintext =
                serde_json::to_string(&super::models::WireGrant::from(oversized.clone())).unwrap();
            (
                mint_envelope(identity(), oversized, &keys(), now(), &FixedNonce).unwrap_err(),
                plaintext,
                ENVELOPE_PREFIX,
            )
        }
        EnvelopeKind::Refresh => {
            let oversized = RefreshCredential {
                refresh_token: SecretString::from(token),
                scope: None,
                expires_in: None,
            };
            let plaintext =
                serde_json::to_string(&super::models::WireRefresh::from(oversized.clone()))
                    .unwrap();
            (
                mint_refresh_envelope(identity(), oversized, &keys(), now(), &FixedNonce)
                    .unwrap_err(),
                plaintext,
                REFRESH_ENVELOPE_PREFIX,
            )
        }
    };
    let expected_size = signed_candidate(kind, &plaintext, prefix).len();
    let EnvelopeMintError::EnvelopeTooLarge {
        size_bytes,
        max_bytes,
    } = error
    else {
        panic!("expected size error");
    };
    assert_eq!(size_bytes, expected_size);
    assert!(size_bytes > super::MAX_ENVELOPE_BYTES);
    assert_eq!(max_bytes, super::MAX_ENVELOPE_BYTES);
}

#[rstest]
#[case::empty("")]
#[case::short("short")]
#[case::just_short("0123456789abcdef0123456789abcde")]
fn opening_rejects_weak_keys(#[case] signing_key: &str) {
    let signing = SecretString::from(signing_key);
    let encryption = SecretString::from("encryption-key");
    let claims = Claims {
        iss: super::ENVELOPE_ISSUER.to_owned(),
        iat: now().timestamp(),
        exp: now()
            .checked_add_signed(chrono::TimeDelta::hours(1))
            .unwrap()
            .timestamp(),
        kind: EnvelopeKind::Access,
        server_id: String::from("server"),
        subject_type: SubjectType::KeyHash,
        subject: String::from("hash"),
        grant: crypto::encrypt_grant(
            r#"{"access_token":"a","token_type":"Bearer"}"#,
            &encryption,
            &FixedNonce,
        )
        .unwrap(),
    };
    let compact = crypto::encode_jwt(&claims, &signing).unwrap();
    let candidate = format!("{ENVELOPE_PREFIX}{compact}");
    let opened = EnvelopeKeys::new(signing, encryption)
        .map(|validated| open_envelope(&candidate, &validated, now()));
    assert!(matches!(opened, Err(EnvelopeMintError::SigningKeyTooShort)));
}

#[test]
fn minimum_keys_can_open_envelopes() {
    let minimum = EnvelopeKeys::new(
        SecretString::from("0123456789abcdef0123456789abcdef"),
        SecretString::from("e"),
    )
    .expect("minimum valid keys");
    let sealed = mint_envelope(identity(), grant(), &minimum, now(), &FixedNonce).unwrap();
    open_envelope(sealed.token.expose_secret(), &minimum, now()).unwrap();
}

#[rstest]
#[case::empty("", (false, false))]
#[case::raw("token", (false, false))]
#[case::access_marker("llm_env_", (true, false))]
#[case::refresh_marker("llm_refresh_", (false, true))]
#[case::access("llm_env_not-a-jwt", (true, false))]
#[case::refresh("llm_refresh_not-a-jwt", (false, true))]
fn routes_by_prefix(#[case] candidate: &str, #[case] expected: (bool, bool)) {
    assert_eq!(candidate.is_envelope(), expected.0);
    assert_eq!(candidate.is_refresh_envelope(), expected.1);
}

#[rstest]
fn replays_the_envelope_corpus(#[files("fuzz/corpus/envelope/*")] path: PathBuf) {
    let signing_keys = keys();
    let bytes = fs::read(path).unwrap();
    if let Ok(candidate) = str::from_utf8(&bytes) {
        drop(open_envelope(candidate, &signing_keys, now()));
        drop(open_refresh_envelope(candidate, &signing_keys, now()));
    }
}

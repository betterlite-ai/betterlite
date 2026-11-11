use std::sync::Arc;

use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, ECDSA_P384_SHA384_FIXED};
use secrecy::ExposeSecret as _;

use crate::gdc_auth::credential::{
    CredentialError, FixtureCredentials, GdchCredentialFactory as _, GdchServiceAccount,
    GdchSigner, JWT_LIFETIME_SECONDS, PemCredentials,
};

use super::support::{
    GARBAGE_PEM, P256_PKCS8, P256_SEC1, P384_PKCS8, P384_SEC1, P521_SEC1, RSA_PKCS8_PEM,
    RSA_PRIVATE_PEM, TOKEN_URI, descriptor, jwt_claims, jwt_header, verify_ecdsa,
};

fn account(pem: &str) -> Arc<GdchServiceAccount> {
    PemCredentials
        .build(descriptor("project-a", pem))
        .expect("a loadable fixture key")
}

#[test]
fn a_sec1_key_signs_the_sdk_claims_with_es256() {
    let token = account(P256_SEC1)
        .sign(1_700_000_000)
        .expect("a signed subject token");
    let header = jwt_header(&token);
    assert_eq!(header.typ.as_deref(), Some("JWT"));
    assert_eq!(header.alg, "ES256");
    assert_eq!(header.kid.as_deref(), Some("key-id"));
    let claims = jwt_claims(&token);
    assert_eq!(
        claims.iss,
        "system:serviceaccount:project-a:service-identity"
    );
    assert_eq!(
        claims.sub,
        "system:serviceaccount:project-a:service-identity"
    );
    assert_eq!(claims.aud, TOKEN_URI);
    assert_eq!(claims.iat, 1_700_000_000);
    assert_eq!(
        claims.exp,
        1_700_000_000_i64.saturating_add(JWT_LIFETIME_SECONDS)
    );
    assert!(verify_ecdsa(P256_PKCS8, &ECDSA_P256_SHA256_FIXED, &token));
}

#[test]
fn a_pkcs8_key_signs_the_same_claims() {
    let token = account(P256_PKCS8)
        .sign(1_700_000_000)
        .expect("a signed subject token");
    assert_eq!(jwt_header(&token).alg, "ES256");
    assert_eq!(
        jwt_claims(&token).iss,
        "system:serviceaccount:project-a:service-identity"
    );
    assert!(verify_ecdsa(P256_PKCS8, &ECDSA_P256_SHA256_FIXED, &token));
}

#[test]
fn a_p384_key_signs_with_es384() {
    let token = account(P384_SEC1)
        .sign(1_700_000_000)
        .expect("a signed subject token");
    assert_eq!(jwt_header(&token).alg, "ES384");
    assert!(verify_ecdsa(P384_PKCS8, &ECDSA_P384_SHA384_FIXED, &token));
}

#[test]
fn a_p521_key_is_not_supported() {
    let error = PemCredentials
        .build(descriptor("project-a", P521_SEC1))
        .unwrap_err();
    assert!(matches!(error, CredentialError::Key));
}

#[test]
fn an_rsa_pkcs8_key_is_not_an_ec_credential() {
    let error = PemCredentials
        .build(descriptor("project-a", RSA_PKCS8_PEM))
        .unwrap_err();
    assert!(matches!(error, CredentialError::Key));
}

#[test]
fn an_rsa_private_key_pem_is_not_an_ec_credential() {
    let error = PemCredentials
        .build(descriptor("project-a", RSA_PRIVATE_PEM))
        .unwrap_err();
    assert!(matches!(error, CredentialError::NotEc));
}

#[test]
fn an_unparsable_ec_private_key_is_refused() {
    let error = PemCredentials
        .build(descriptor("project-a", GARBAGE_PEM))
        .unwrap_err();
    assert!(matches!(error, CredentialError::Key));
}

#[test]
fn an_encrypted_private_key_is_not_an_ec_credential() {
    let pem = "-----BEGIN ENCRYPTED PRIVATE KEY-----\nbm90IGEga2V5\n-----END ENCRYPTED PRIVATE KEY-----\n";
    let error = PemCredentials
        .build(descriptor("project-a", pem))
        .unwrap_err();
    assert!(matches!(error, CredentialError::NotEc));
}

#[test]
fn a_truncated_pkcs8_key_is_refused() {
    let truncated = P256_PKCS8.replace("-----END PRIVATE KEY-----", "");
    let error = PemCredentials
        .build(descriptor("project-a", &truncated))
        .unwrap_err();
    assert!(matches!(error, CredentialError::Pem));
}

#[test]
fn a_key_body_never_reaches_the_error_text() {
    let secret_body = "c2VjcmV0LWtleS1tYXRlcmlhbA==";
    let pem =
        format!("-----BEGIN EC PRIVATE KEY-----\n{secret_body}\n-----END EC PRIVATE KEY-----\n");
    let error = PemCredentials
        .build(descriptor("project-a", &pem))
        .unwrap_err();
    assert!(!error.to_string().contains(secret_body));
    assert!(!format!("{error:?}").contains(secret_body));
}

#[test]
fn the_pem_factory_loads_the_key_and_the_fixture_factory_does_not() {
    let loaded = PemCredentials
        .build(descriptor("project-a", P256_SEC1))
        .expect("a loaded credential");
    let _token = loaded.sign(1_700_000_000).expect("a signed subject token");
    let error = FixtureCredentials {
        token: secrecy::SecretString::from("fixture".to_owned()),
    }
    .build(descriptor("project-a", P256_SEC1))
    .expect("a fixture credential")
    .sign(1_700_000_000)
    .unwrap_err();
    assert!(matches!(error, CredentialError::Fixture));
}

#[test]
fn a_fixture_credential_keeps_the_descriptor() {
    let fixture = FixtureCredentials {
        token: secrecy::SecretString::from("fixture-token".to_owned()),
    }
    .build(descriptor("project-a", P256_SEC1))
    .expect("a fixture credential");
    assert_eq!(fixture.descriptor.project, "project-a");
    let GdchSigner::Fixture(token) = &fixture.signer else {
        panic!("expected a fixture signer");
    };
    assert_eq!(token.expose_secret(), "fixture-token");
}

fn certificate_pem_of_length(length: usize) -> String {
    let header = "-----BEGIN CERTIFICATE-----\n";
    let footer = "-----END CERTIFICATE-----\n";
    let fixed = header
        .len()
        .saturating_add(footer.len())
        .saturating_add(16_000);
    let padding = length.saturating_sub(fixed);
    let body = "A".repeat(16_000);
    let spaces = " ".repeat(padding);
    format!("{header}{body}{spaces}{footer}")
}

#[test]
fn the_pem_size_limit_keeps_the_boundary_and_refuses_one_byte_more() {
    let at_limit = certificate_pem_of_length(0x4000);
    assert_eq!(at_limit.len(), 0x4000);
    let at_limit_error = PemCredentials
        .build(descriptor("project-a", &at_limit))
        .unwrap_err();
    assert!(matches!(at_limit_error, CredentialError::NotEc));
    let over_limit = certificate_pem_of_length(0x4001);
    let over_error = PemCredentials
        .build(descriptor("project-a", &over_limit))
        .unwrap_err();
    assert!(matches!(over_error, CredentialError::Pem));
}

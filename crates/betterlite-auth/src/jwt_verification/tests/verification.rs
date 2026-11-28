use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::jwt_verification::decode::JwtHeader;
use crate::jwt_verification::errors::VerificationError;
use crate::jwt_verification::errors::into_proxy_exception;
use crate::jwt_verification::keys::PublicKey;
use crate::jwt_verification::verifier::DecodeParams;

use super::fixtures;
use super::support::{
    Harness, ScriptedSource, SourceReply, audience_one, capture_logs, claim_fields, harness,
    settings, text_claim,
};

fn scoped() -> Harness {
    harness(settings(
        Some("litellm-proxy"),
        None,
        JwtAuthSettings::default(),
    ))
}

async fn verifies(algorithm: &str, token: &str, jwk: &str) {
    let header = JwtHeader::parse(token).unwrap();
    assert_eq!(header.algorithm.as_deref(), Some(algorithm));
    let harness = scoped();
    let source = ScriptedSource::with_keys(jwk);
    let payload = harness.verifier.auth_jwt(token, &source).await.unwrap();
    assert_eq!(text_claim(&claim_fields(&payload), "sub"), Some("user-1"));
}

#[tokio::test]
async fn valid_signature() {
    let harness = scoped();
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let payload = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &source)
        .await
        .unwrap();
    let fields = claim_fields(&payload);
    assert_eq!(text_claim(&fields, "sub"), Some("user-1"));
    assert_eq!(text_claim(&fields, "aud"), Some("litellm-proxy"));
    assert_eq!(source.calls(), vec!["lookup:rsa-key".to_owned()]);
}

#[tokio::test]
async fn header_is_debug_logged_before_verification() {
    let (logs, _guard) = capture_logs();
    let harness = scoped();
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &source)
        .await
        .unwrap();
    let expected = "DEBUG LiteLLM Proxy header: alg=Some(\"RS256\") kid=Some(\"rsa-key\")";
    assert!(
        logs.take().iter().any(|line| line.as_str() == expected),
        "the unverified header was not debug-logged"
    );
}

#[tokio::test]
async fn rs256() {
    verifies("RS256", fixtures::TOKEN_RS256, fixtures::RSA_JWK).await;
}

#[tokio::test]
async fn rs384() {
    verifies("RS384", fixtures::TOKEN_RS384, fixtures::RSA_JWK).await;
}

#[tokio::test]
async fn rs512() {
    verifies("RS512", fixtures::TOKEN_RS512, fixtures::RSA_JWK).await;
}

#[tokio::test]
async fn ps256() {
    verifies("PS256", fixtures::TOKEN_PS256, fixtures::RSA_JWK).await;
}

#[tokio::test]
async fn ps384() {
    verifies("PS384", fixtures::TOKEN_PS384, fixtures::RSA_JWK).await;
}

#[tokio::test]
async fn ps512() {
    verifies("PS512", fixtures::TOKEN_PS512, fixtures::RSA_JWK).await;
}

#[tokio::test]
async fn es256() {
    verifies("ES256", fixtures::TOKEN_ES256, fixtures::P256_JWK).await;
}

#[tokio::test]
async fn es384() {
    verifies("ES384", fixtures::TOKEN_ES384, fixtures::P384_JWK).await;
}

#[tokio::test]
async fn es512() {
    verifies("ES512", fixtures::TOKEN_ES512, fixtures::P521_JWK).await;
}

#[tokio::test]
async fn eddsa() {
    verifies("EdDSA", fixtures::TOKEN_EDDSA, fixtures::ED_JWK).await;
}

#[tokio::test]
async fn wrong_signature() {
    let harness = scoped();
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_WRONG_SIGNATURE, &source)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: Signature verification failed"
    );
    let exception = into_proxy_exception(&error);
    assert_eq!(exception.code, "401");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(
        exception.message,
        "Authentication Error, Validation fails: Signature verification failed"
    );
}

async fn rejects_algorithm(harness: &Harness, source: &ScriptedSource, token: &str) {
    let error = harness.verifier.auth_jwt(token, source).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: The specified alg value is not allowed"
    );
    assert_eq!(into_proxy_exception(&error).code, "401");
}

#[tokio::test]
async fn algorithm_confusion() {
    let harness = scoped();
    let source = ScriptedSource::with_global(vec![
        SourceReply::Key(fixtures::RSA_JWK.to_owned()),
        SourceReply::Key(fixtures::RSA_JWK.to_owned()),
    ]);
    rejects_algorithm(&harness, &source, fixtures::TOKEN_HS256).await;
    rejects_algorithm(&harness, &source, fixtures::TOKEN_NONE).await;
    assert_eq!(
        source.calls(),
        vec!["lookup:None".to_owned(), "lookup:None".to_owned()]
    );
}

#[tokio::test]
async fn wrong_audience() {
    let harness = scoped();
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_WRONG_AUDIENCE, &source)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Validation fails: Invalid audience");
    assert_eq!(into_proxy_exception(&error).code, "401");
}

#[tokio::test]
async fn wrong_issuer() {
    let harness = harness(settings(
        Some("litellm-proxy"),
        Some(fixtures::ISSUER_A),
        JwtAuthSettings::default(),
    ));
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_WRONG_ISSUER, &source)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Validation fails: Invalid issuer");
}

#[tokio::test]
async fn expired_token() {
    let harness = scoped();
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_EXPIRED, &source)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Token Expired");
    assert!(matches!(error, VerificationError::Expired));
}

#[test]
fn pem_certificate() {
    let harness = scoped();
    let key = PublicKey::Certificate(fixtures::RSA_CERTIFICATE);
    let audience = Some(audience_one("litellm-proxy"));
    let params = DecodeParams {
        audience: audience.as_ref(),
        issuer: None,
        options: None,
        disable_audience_validation: false,
    };
    let payload = harness
        .verifier
        .decode_with_public_key(fixtures::TOKEN_RS256, &key, &params)
        .unwrap();
    assert_eq!(text_claim(&claim_fields(&payload), "sub"), Some("user-1"));
}

#[test]
fn raw_pem_rejected_x509_accepted() {
    let harness = scoped();
    let audience = Some(audience_one("litellm-proxy"));
    let params = DecodeParams {
        audience: audience.as_ref(),
        issuer: None,
        options: None,
        disable_audience_validation: false,
    };
    let raw = PublicKey::Certificate(fixtures::RSA_PUBLIC_KEY);
    let error = harness
        .verifier
        .decode_with_public_key(fixtures::TOKEN_RS256, &raw, &params)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: the configured public key is not a PEM X.509 certificate"
    );
    let certificate = PublicKey::Certificate(fixtures::RSA_CERTIFICATE);
    harness
        .verifier
        .decode_with_public_key(fixtures::TOKEN_RS256, &certificate, &params)
        .unwrap();
}

#[tokio::test]
async fn jwk_allowed_field_projection() {
    let harness = scoped();
    let mut projected: IndexMap<String, Json> = serde_json::from_str(fixtures::RSA_JWK).unwrap();
    projected.insert("alg".to_owned(), Json::Text("HS256".to_owned()));
    projected.insert("use".to_owned(), Json::Text("encryption".to_owned()));
    projected.insert(
        "x5c".to_owned(),
        Json::List(vec![Json::Text("unused".to_owned())]),
    );
    let source = ScriptedSource::with_global(vec![SourceReply::Key(
        serde_json::to_string(&projected).unwrap(),
    )]);
    harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &source)
        .await
        .unwrap();

    let mut incomplete: IndexMap<String, Json> = serde_json::from_str(fixtures::RSA_JWK).unwrap();
    incomplete.shift_remove("kty");
    let incomplete_source = ScriptedSource::with_global(vec![SourceReply::Key(
        serde_json::to_string(&incomplete).unwrap(),
    )]);
    let error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &incomplete_source)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Validation fails: the configured key is not compatible with the token algorithm"
    );
}

#[tokio::test]
async fn fetch_errors_not_signature_wrapped() {
    let harness = scoped();
    let outage_source = ScriptedSource::with_global(vec![SourceReply::Unreachable]);
    let outage_error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &outage_source)
        .await
        .unwrap_err();
    assert!(!outage_error.to_string().contains("Validation fails"));
    let unavailable = into_proxy_exception(&outage_error);
    assert_eq!(unavailable.code, "503");
    assert_eq!(unavailable.error_type, "auth_provider_unavailable");
    assert_eq!(unavailable.param.as_deref(), Some("None"));

    let missing_source = ScriptedSource::with_global(vec![SourceReply::NoMatchingKey]);
    let missing_error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &missing_source)
        .await
        .unwrap_err();
    assert!(!missing_error.to_string().contains("Validation fails"));
    let refused = into_proxy_exception(&missing_error);
    assert_eq!(refused.code, "401");
    assert_eq!(refused.error_type, "auth_error");
    assert!(
        refused
            .message
            .starts_with("Authentication Error, No matching public key found.")
    );
}

#[tokio::test]
async fn null_public_key_error() {
    let harness = scoped();
    let source = ScriptedSource::with_global(vec![SourceReply::Null]);
    let error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &source)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Invalid JWT Submitted");
    let exception = into_proxy_exception(&error);
    assert_eq!(
        exception.message,
        "Authentication Error, Invalid JWT Submitted"
    );
    assert_eq!(exception.code, "401");
    assert_eq!(exception.error_type, "auth_error");
}

#[tokio::test]
async fn expiry_error_fields_and_validation_prefix() {
    let harness = scoped();
    let expired_source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let expired_error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_EXPIRED, &expired_source)
        .await
        .unwrap_err();
    let expired = into_proxy_exception(&expired_error);
    assert_eq!(expired.message, "Token Expired");
    assert_eq!(expired.error_type, "expired_key");
    assert!(expired.param.is_none());
    assert_eq!(expired.code, "401");

    let refused_source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let refused_error = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_WRONG_AUDIENCE, &refused_source)
        .await
        .unwrap_err();
    let refused = into_proxy_exception(&refused_error);
    assert_eq!(
        refused.message,
        "Authentication Error, Validation fails: Invalid audience"
    );
    assert_eq!(refused.error_type, "auth_error");
    assert_eq!(refused.param.as_deref(), Some("None"));
    assert_eq!(refused.code, "401");
}

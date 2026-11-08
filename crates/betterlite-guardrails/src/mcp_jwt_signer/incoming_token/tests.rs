#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;

use axum::Router;
use axum::http::{StatusCode, Uri};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use betterlite_prompts::arguments::json::{Json, Object};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use secrecy::SecretString;
use serde_json::Number;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{IncomingTokenError, verify_incoming_jwt};
use crate::mcp_jwt_signer::JwtSigner;
use crate::mcp_jwt_signer::jwks_document::get_jwks;
use crate::mcp_jwt_signer::signer_settings::SignerOptions;
use crate::mcp_jwt_signer::signing_key::SigningKey;
use crate::mcp_token_claims::Claims;

type Server = JoinSet<Result<(), io::Error>>;

const ISSUER: &str = "https://idp.example";
const AUDIENCE: &str = "api://app";

async fn serve(discovery_document: &str, jwks_document: &str) -> (String, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let discovery = discovery_document.replace("$BASE", &base);
    let jwks = jwks_document.to_owned();
    let handler = move |uri: Uri| {
        let discovery_answer = discovery.clone();
        let jwks_answer = jwks.clone();
        async move {
            match uri.path() {
                "/discovery" => (StatusCode::OK, discovery_answer),
                "/jwks" => (StatusCode::OK, jwks_answer),
                _ => (StatusCode::NOT_FOUND, String::new()),
            }
        }
    };
    let router = Router::new().fallback(handler);
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

async fn idp_server(jwks: String) -> (String, Server) {
    serve(r#"{"jwks_uri":"$BASE/jwks"}"#, &jwks).await
}

fn idp_key() -> SigningKey {
    SigningKey::load(&BTreeMap::new()).expect("idp key")
}

fn jwks_body(key: &SigningKey) -> String {
    serde_json::to_string(&get_jwks(key)).expect("jwks json")
}

fn jwks_body_with_algorithm(key: &SigningKey, algorithm: &str) -> String {
    let entry = &get_jwks(key).keys[0];
    format!(
        r#"{{"keys":[{{"kty":"RSA","alg":"{algorithm}","use":"sig","kid":"{}","n":"{}","e":"{}"}}]}}"#,
        entry.kid, entry.modulus, entry.exponent
    )
}

fn base_options() -> SignerOptions {
    SignerOptions {
        issuer: None,
        audience: None,
        ttl_seconds: None,
        access_token_discovery_uri: None,
        token_introspection_endpoint: None,
        verify_issuer: None,
        verify_audience: None,
        end_user_claim_sources: None,
        add_claims: None,
        set_claims: None,
        remove_claims: None,
        channel_token_audience: None,
        channel_token_ttl: None,
        required_claims: None,
        optional_claims: None,
        debug_headers: false,
        allowed_scopes: None,
        timeout: None,
    }
}

fn options(base: &str) -> SignerOptions {
    let mut options = base_options();
    options.access_token_discovery_uri = Some(format!("{base}/discovery"));
    options
}

fn build_signer(options: &SignerOptions) -> JwtSigner {
    JwtSigner::new(options, &BTreeMap::new(), reqwest::Client::default()).expect("signer")
}

fn now() -> i64 {
    i64::try_from(jsonwebtoken::get_current_timestamp()).expect("clock")
}

fn claims() -> Claims {
    let now = now();
    Object::from([
        ("iss".to_owned(), Json::Text(ISSUER.to_owned())),
        ("aud".to_owned(), Json::Text(AUDIENCE.to_owned())),
        ("sub".to_owned(), Json::Text("alice".to_owned())),
        (
            "exp".to_owned(),
            Json::Number(Number::from(now.saturating_add(300))),
        ),
        ("iat".to_owned(), Json::Number(Number::from(now))),
    ])
}

fn token(key: &SigningKey, claims: &Claims) -> SecretString {
    key.sign(claims).expect("signed token")
}

fn token_with_header(
    key: &SigningKey,
    claims: &Claims,
    kid: Option<&str>,
    algorithm: Algorithm,
) -> SecretString {
    let mut header = Header::new(algorithm);
    header.kid = kid.map(str::to_owned);
    let token = jsonwebtoken::encode(&header, claims, &key.encoding).expect("token");
    SecretString::from(token)
}

fn unsigned_token(claims: &Claims, kid: &str) -> SecretString {
    let header = URL_SAFE_NO_PAD.encode(format!(r#"{{"alg":"none","kid":"{kid}"}}"#));
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("payload"));
    SecretString::from(format!("{header}.{payload}."))
}

fn hs256_token(claims: &Claims, kid: &str) -> SecretString {
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(kid.to_owned());
    let token = jsonwebtoken::encode(&header, claims, &EncodingKey::from_secret(b"attacker"))
        .expect("token");
    SecretString::from(token)
}

#[tokio::test]
async fn verifies_a_token_signed_by_the_idp_key() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let claims = claims();
    let verified =
        verify_incoming_jwt(&build_signer(&options(&base)), &token(&key, &claims), now())
            .await
            .expect("verified");
    assert_eq!(verified.get("sub"), claims.get("sub"));
}

#[tokio::test]
async fn uses_the_first_key_when_the_token_has_no_kid() {
    let key = idp_key();
    let decoy = idp_key();
    let jwks = format!(r#"{{"keys":[{},{}]}}"#, key_entry(&key), key_entry(&decoy));
    let (base, _server) = idp_server(jwks).await;
    let claims = claims();
    let token = token_with_header(&key, &claims, None, Algorithm::RS256);
    let verified = verify_incoming_jwt(&build_signer(&options(&base)), &token, now())
        .await
        .expect("verified");
    assert_eq!(verified.get("sub"), claims.get("sub"));
}

fn key_entry(key: &SigningKey) -> String {
    let entry = &get_jwks(key).keys[0];
    format!(
        r#"{{"kty":"RSA","alg":"RS256","use":"sig","kid":"{}","n":"{}","e":"{}"}}"#,
        entry.kid, entry.modulus, entry.exponent
    )
}

#[tokio::test]
async fn rejects_a_token_whose_kid_matches_no_key() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let claims = claims();
    let token = token_with_header(&key, &claims, Some("unknown"), Algorithm::RS256);
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token, now())
        .await
        .expect_err("unknown kid");
    assert!(matches!(error, IncomingTokenError::MissingKey { .. }));
    assert_eq!(
        error.to_string(),
        format!("No JWKS key matching kid=unknown at {base}/jwks")
    );
}

#[tokio::test]
async fn rejects_a_token_with_a_bad_signature() {
    let key = idp_key();
    let attacker = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let claims = claims();
    let token = token_with_header(&attacker, &claims, Some(&key.kid), Algorithm::RS256);
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token, now())
        .await
        .expect_err("bad signature");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
}

#[tokio::test]
async fn rejects_an_expired_token() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let mut claims = claims();
    claims.insert(
        "exp".to_owned(),
        Json::Number(Number::from(now().saturating_sub(10))),
    );
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token(&key, &claims), now())
        .await
        .expect_err("expired");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
}

#[tokio::test]
async fn rejects_a_token_that_is_not_yet_valid() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let mut claims = claims();
    claims.insert(
        "nbf".to_owned(),
        Json::Number(Number::from(now().saturating_add(3600))),
    );
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token(&key, &claims), now())
        .await
        .expect_err("not yet valid");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
}

#[tokio::test]
async fn rejects_a_token_whose_iat_is_in_the_future() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let mut claims = claims();
    claims.insert(
        "iat".to_owned(),
        Json::Number(Number::from(now().saturating_add(3600))),
    );
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token(&key, &claims), now())
        .await
        .expect_err("future iat");
    assert!(matches!(error, IncomingTokenError::NotYetValid));
}

#[tokio::test]
async fn accepts_a_fractional_issued_at() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let mut accepted = claims();
    accepted.insert(
        "iat".to_owned(),
        serde_json::from_str::<Json>("1700000000.5").expect("fractional number"),
    );
    verify_incoming_jwt(
        &build_signer(&options(&base)),
        &token(&key, &accepted),
        now(),
    )
    .await
    .expect("a fractional iat is a valid NumericDate");
    let mut rejected = claims();
    let future = format!("{}.5", now().saturating_add(3600));
    rejected.insert(
        "iat".to_owned(),
        serde_json::from_str::<Json>(&future).expect("fractional number"),
    );
    let error = verify_incoming_jwt(
        &build_signer(&options(&base)),
        &token(&key, &rejected),
        now(),
    )
    .await
    .expect_err("future fractional iat");
    assert!(matches!(error, IncomingTokenError::NotYetValid));
}

#[tokio::test]
async fn rejects_a_token_with_a_non_string_subject() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let mut claims = claims();
    claims.insert("sub".to_owned(), Json::Number(Number::from(42_i32)));
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token(&key, &claims), now())
        .await
        .expect_err("numeric subject");
    assert!(matches!(
        error,
        IncomingTokenError::ClaimType { claim: "sub", .. }
    ));
}

#[tokio::test]
async fn the_audience_is_checked_only_when_configured() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let claims = claims();
    let token = token(&key, &claims);
    verify_incoming_jwt(&build_signer(&options(&base)), &token, now())
        .await
        .expect("no audience check");
    let mut configured = options(&base);
    configured.verify_audience = Some("other-audience".to_owned());
    let error = verify_incoming_jwt(&build_signer(&configured), &token, now())
        .await
        .expect_err("wrong audience");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
    configured.verify_audience = Some(AUDIENCE.to_owned());
    verify_incoming_jwt(&build_signer(&configured), &token, now())
        .await
        .expect("matching audience");
}

#[tokio::test]
async fn a_configured_audience_requires_the_audience_claim() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let mut claims = claims();
    claims.shift_remove("aud");
    let mut configured = options(&base);
    configured.verify_audience = Some(AUDIENCE.to_owned());
    let error = verify_incoming_jwt(&build_signer(&configured), &token(&key, &claims), now())
        .await
        .expect_err("missing audience");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
}

#[tokio::test]
async fn the_issuer_is_checked_only_when_configured() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let token = token(&key, &claims());
    let mut configured = options(&base);
    configured.verify_issuer = Some("https://other.example".to_owned());
    let error = verify_incoming_jwt(&build_signer(&configured), &token, now())
        .await
        .expect_err("wrong issuer");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
    configured.verify_issuer = Some(ISSUER.to_owned());
    verify_incoming_jwt(&build_signer(&configured), &token, now())
        .await
        .expect("matching issuer");
}

#[tokio::test]
async fn a_configured_issuer_requires_the_issuer_claim() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let mut claims = claims();
    claims.shift_remove("iss");
    let mut configured = options(&base);
    configured.verify_issuer = Some(ISSUER.to_owned());
    let error = verify_incoming_jwt(&build_signer(&configured), &token(&key, &claims), now())
        .await
        .expect_err("missing issuer");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
}

#[tokio::test]
async fn the_algorithm_comes_from_the_jwk_and_not_from_the_token_header() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body_with_algorithm(&key, "RS384")).await;
    let claims = claims();
    let wrong_header = token_with_header(&key, &claims, Some(&key.kid), Algorithm::RS256);
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &wrong_header, now())
        .await
        .expect_err("header algorithm not allowed");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
    let matching = token_with_header(&key, &claims, Some(&key.kid), Algorithm::RS384);
    verify_incoming_jwt(&build_signer(&options(&base)), &matching, now())
        .await
        .expect("matching algorithm");
}

#[tokio::test]
async fn rejects_an_unsigned_algorithm_none_token() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let token = unsigned_token(&claims(), &key.kid);
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token, now())
        .await
        .expect_err("alg none");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
}

#[tokio::test]
async fn rejects_an_hs256_token_signed_with_the_public_key() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let token = hs256_token(&claims(), &key.kid);
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token, now())
        .await
        .expect_err("hs256 confusion");
    assert!(matches!(error, IncomingTokenError::Verification { .. }));
}

#[tokio::test]
async fn a_discovery_document_without_a_jwks_uri_is_an_error() {
    let (base, _server) = serve("{}", "{}").await;
    let error = verify_incoming_jwt(
        &build_signer(&options(&base)),
        &SecretString::from("a.b.c"),
        now(),
    )
    .await
    .expect_err("no jwks uri");
    assert!(matches!(error, IncomingTokenError::MissingJwksUri { .. }));
    assert_eq!(
        error.to_string(),
        format!("discovery document at {base}/discovery has no 'jwks_uri'")
    );
}

#[tokio::test]
async fn a_key_set_that_cannot_be_parsed_is_an_error() {
    let (base, _server) = serve(
        r#"{"jwks_uri":"$BASE/jwks"}"#,
        r#"{"keys":[{"kty":"bogus","kid":"abc"}]}"#,
    )
    .await;
    let token = unsigned_token(&claims(), "abc");
    let error = verify_incoming_jwt(&build_signer(&options(&base)), &token, now())
        .await
        .expect_err("bad key set");
    assert!(matches!(error, IncomingTokenError::NoUsableKey { .. }));
    assert!(
        error
            .to_string()
            .starts_with(&format!("Failed to parse JWKS from {base}/jwks: "))
    );
}

#[tokio::test]
async fn an_unusable_key_next_to_a_good_one_is_skipped() {
    let key = idp_key();
    let entry = &get_jwks(&key).keys[0];
    let jwks = format!(
        r#"{{"keys":[{{"kty":"RSA","alg":"RSA-OAEP","use":"enc","kid":"encryption","n":"{}","e":"{}"}},{{"kty":"RSA","alg":"ML-DSA-44","kid":"post-quantum","n":"{}","e":"{}"}},{{"kty":"EC","crv":"P-521","kid":"es512","x":"AQAB","y":"AQAB"}},{}]}}"#,
        entry.modulus,
        entry.exponent,
        entry.modulus,
        entry.exponent,
        key_entry(&key)
    );
    let (base, _server) = idp_server(jwks).await;
    let claims = claims();
    let token = token_with_header(&key, &claims, None, Algorithm::RS256);
    let verified = verify_incoming_jwt(&build_signer(&options(&base)), &token, now())
        .await
        .expect("first usable key");
    assert_eq!(verified.get("sub"), claims.get("sub"));
}

#[tokio::test]
async fn a_token_header_that_is_not_base64url_is_an_error() {
    let (base, _server) = idp_server("{}".to_owned()).await;
    let error = verify_incoming_jwt(
        &build_signer(&options(&base)),
        &SecretString::from("not base64!.a.b"),
        now(),
    )
    .await
    .expect_err("bad header");
    assert!(matches!(error, IncomingTokenError::HeaderBase64 { .. }));
}

#[tokio::test]
async fn the_verified_claims_keep_every_claim() {
    let key = idp_key();
    let (base, _server) = idp_server(jwks_body(&key)).await;
    let mut claims = claims();
    claims.insert(
        "groups".to_owned(),
        Json::List(vec![Json::Text("eng".to_owned())]),
    );
    let verified =
        verify_incoming_jwt(&build_signer(&options(&base)), &token(&key, &claims), now())
            .await
            .expect("verified");
    assert_eq!(verified, claims);
}

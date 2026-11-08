#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;

use axum::Router;
use axum::http::{StatusCode, Uri};
use betterlite_prompts::arguments::json::{Json, Object};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{UpstreamRequest, inject_upstream_headers};
use crate::mcp_jwt_signer::JwtSigner;
use crate::mcp_jwt_signer::jwks_document::get_jwks;
use crate::mcp_jwt_signer::signer_settings::SignerOptions;
use crate::mcp_jwt_signer::signing_key::SigningKey;
use crate::mcp_token_claims::{AuthFields, Claims};

type Server = JoinSet<Result<(), io::Error>>;

const NOW: i64 = 1_700_000_000;

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

fn idp_key() -> SigningKey {
    SigningKey::load(&BTreeMap::new()).expect("idp key")
}

fn jwks_body(key: &SigningKey) -> String {
    serde_json::to_string(&get_jwks(key)).expect("jwks json")
}

fn build_signer(options: &SignerOptions) -> JwtSigner {
    JwtSigner::new(options, &BTreeMap::new(), reqwest::Client::default()).expect("signer")
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

fn with_discovery(base: &str) -> SignerOptions {
    let mut options = base_options();
    options.access_token_discovery_uri = Some(format!("{base}/discovery"));
    options
}

fn auth() -> AuthFields {
    AuthFields {
        user_id: None,
        user_email: None,
        end_user_id: None,
        team_id: None,
        org_id: None,
        token: None,
        api_key: None,
    }
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn bearer_token(headers: &BTreeMap<String, String>) -> String {
    headers["Authorization"]
        .strip_prefix("Bearer ")
        .expect("bearer prefix")
        .to_owned()
}

fn decode_claims(signer: &JwtSigner, token: &str) -> Claims {
    let entry = &get_jwks(&signer.key).keys[0];
    let decoding =
        DecodingKey::from_rsa_components(&entry.modulus, &entry.exponent).expect("decoding key");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.required_spec_claims.clear();
    validation.validate_aud = false;
    jsonwebtoken::decode::<Claims>(token, &decoding, &validation)
        .expect("verified token")
        .claims
}

fn idp_token(key: &SigningKey, subject: &str) -> SecretString {
    let claims = Object::from([
        ("sub".to_owned(), Json::Text(subject.to_owned())),
        (
            "exp".to_owned(),
            Json::Number(Number::from(
                i64::try_from(jsonwebtoken::get_current_timestamp())
                    .expect("clock")
                    .saturating_add(300),
            )),
        ),
    ]);
    key.sign(&claims).expect("idp token")
}

#[tokio::test]
async fn without_auth_fields_the_extra_headers_come_back_unchanged() {
    let signer = build_signer(&SignerOptions::default());
    let extra = headers(&[("X-Trace", "abc")]);
    let request = UpstreamRequest {
        auth: None,
        jwt_claims: None,
        extra_headers: Some(&extra),
        raw_headers: None,
        for_list_tools: false,
        tool_name: "weather",
        now: NOW,
    };
    let merged = inject_upstream_headers(&signer, &request)
        .await
        .expect("unchanged");
    assert_eq!(merged, extra);
}

#[tokio::test]
async fn the_bearer_token_is_read_case_insensitively_from_the_authorization_header() {
    let key = idp_key();
    let (base, _server) = serve(r#"{"jwks_uri":"$BASE/jwks"}"#, &jwks_body(&key)).await;
    let signer = build_signer(&with_discovery(&base));
    let auth = auth();
    let token = idp_token(&key, "alice");
    let raw = headers(&[(
        "AUTHORIZATION",
        &format!("BEARER {}", token.expose_secret()),
    )]);
    let request = UpstreamRequest {
        auth: Some(&auth),
        jwt_claims: None,
        extra_headers: None,
        raw_headers: Some(&raw),
        for_list_tools: false,
        tool_name: "weather",
        now: NOW,
    };
    let merged = inject_upstream_headers(&signer, &request)
        .await
        .expect("signed");
    let claims = decode_claims(&signer, &bearer_token(&merged));
    assert_eq!(claims.get("sub"), Some(&Json::Text("alice".to_owned())));
}

#[tokio::test]
async fn a_non_bearer_authorization_header_yields_no_token() {
    let key = idp_key();
    let (base, _server) = serve(r#"{"jwks_uri":"$BASE/jwks"}"#, &jwks_body(&key)).await;
    let signer = build_signer(&with_discovery(&base));
    let auth = auth();
    let raw = headers(&[("authorization", "Basic dXNlcjpwYXNz")]);
    let proxy_claims = Object::from([("sub".to_owned(), Json::Text("bob".to_owned()))]);
    let request = UpstreamRequest {
        auth: Some(&auth),
        jwt_claims: Some(&proxy_claims),
        extra_headers: None,
        raw_headers: Some(&raw),
        for_list_tools: false,
        tool_name: "weather",
        now: NOW,
    };
    let merged = inject_upstream_headers(&signer, &request)
        .await
        .expect("signed");
    let claims = decode_claims(&signer, &bearer_token(&merged));
    assert_eq!(claims.get("sub"), Some(&Json::Text("bob".to_owned())));
}

#[tokio::test]
async fn an_empty_bearer_token_counts_as_no_token() {
    let key = idp_key();
    let (base, _server) = serve(r#"{"jwks_uri":"$BASE/jwks"}"#, &jwks_body(&key)).await;
    let signer = build_signer(&with_discovery(&base));
    let auth = auth();
    let raw = headers(&[("authorization", "Bearer ")]);
    let proxy_claims = Object::from([("sub".to_owned(), Json::Text("bob".to_owned()))]);
    let request = UpstreamRequest {
        auth: Some(&auth),
        jwt_claims: Some(&proxy_claims),
        extra_headers: None,
        raw_headers: Some(&raw),
        for_list_tools: false,
        tool_name: "weather",
        now: NOW,
    };
    let merged = inject_upstream_headers(&signer, &request)
        .await
        .expect("signed");
    let claims = decode_claims(&signer, &bearer_token(&merged));
    assert_eq!(claims.get("sub"), Some(&Json::Text("bob".to_owned())));
}

#[tokio::test]
async fn list_tools_requests_are_signed_with_the_list_scope() {
    let signer = build_signer(&SignerOptions::default());
    let auth = auth();
    let extra = headers(&[("X-Trace", "abc")]);
    let request = UpstreamRequest {
        auth: Some(&auth),
        jwt_claims: None,
        extra_headers: Some(&extra),
        raw_headers: None,
        for_list_tools: true,
        tool_name: "weather",
        now: NOW,
    };
    let merged = inject_upstream_headers(&signer, &request)
        .await
        .expect("signed");
    assert_eq!(merged["X-Trace"], "abc");
    let claims = decode_claims(&signer, &bearer_token(&merged));
    assert_eq!(
        claims.get("scope"),
        Some(&Json::Text("mcp:tools/list".to_owned()))
    );
}

#[tokio::test]
async fn call_requests_are_signed_with_the_tool_scope() {
    let signer = build_signer(&SignerOptions::default());
    let auth = auth();
    let request = UpstreamRequest {
        auth: Some(&auth),
        jwt_claims: None,
        extra_headers: None,
        raw_headers: None,
        for_list_tools: false,
        tool_name: "weather",
        now: NOW,
    };
    let merged = inject_upstream_headers(&signer, &request)
        .await
        .expect("signed");
    let claims = decode_claims(&signer, &bearer_token(&merged));
    assert_eq!(
        claims.get("scope"),
        Some(&Json::Text(
            "mcp:tools/call mcp:tools/weather:call".to_owned()
        ))
    );
}

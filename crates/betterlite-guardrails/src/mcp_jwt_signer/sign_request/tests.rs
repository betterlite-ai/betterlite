#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{StatusCode, Uri};
use betterlite_prompts::arguments::json::{Json, Object};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tracing::subscriber::set_default;

use super::{HookData, PreCallInput, SignError, sign};
use crate::mcp_guardrail_translation::mapping_registration::McpCallType;
use crate::mcp_jwt_signer::JwtSigner;
use crate::mcp_jwt_signer::jwks_document::get_jwks;
use crate::mcp_jwt_signer::signer_settings::SignerOptions;
use crate::mcp_jwt_signer::signing_key::SigningKey;
use crate::mcp_token_claims::{AuthFields, Claims};

type Server = JoinSet<Result<(), io::Error>>;

const NOW: i64 = 1_700_000_000;
const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "mcp";

#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

async fn serve(
    discovery_document: &str,
    jwks_document: &str,
    introspection_document: &str,
) -> (String, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let discovery = discovery_document.replace("$BASE", &base);
    let jwks = jwks_document.to_owned();
    let introspection = introspection_document.to_owned();
    let handler = move |uri: Uri| {
        let discovery_answer = discovery.clone();
        let jwks_answer = jwks.clone();
        let introspection_answer = introspection.clone();
        async move {
            match uri.path() {
                "/discovery" => (StatusCode::OK, discovery_answer),
                "/jwks" => (StatusCode::OK, jwks_answer),
                "/introspect" => (StatusCode::OK, introspection_answer),
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

fn build_signer(options: &SignerOptions) -> JwtSigner {
    JwtSigner::new(options, &BTreeMap::new(), reqwest::Client::default()).expect("signer")
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

fn data(
    tool_name: Option<&str>,
    token: Option<&str>,
    extra_headers: Option<BTreeMap<String, String>>,
) -> HookData {
    HookData {
        tool_name: tool_name.map(str::to_owned),
        incoming_bearer_token: token.map(SecretString::from),
        extra_headers,
    }
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn input<'input>(
    auth: &'input AuthFields,
    data: &'input HookData,
    call_type: McpCallType,
) -> PreCallInput<'input> {
    PreCallInput {
        auth,
        jwt_claims: None,
        data,
        call_type: Some(call_type),
        now: NOW,
    }
}

fn decode_claims(signer: &JwtSigner, token: &str) -> Claims {
    let entry = &get_jwks(&signer.key).keys[0];
    let decoding =
        DecodingKey::from_rsa_components(&entry.modulus, &entry.exponent).expect("decoding key");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.required_spec_claims.clear();
    validation.validate_aud = false;
    // the signer is driven by the fixed test clock, which is in the past
    validation.validate_exp = false;
    jsonwebtoken::decode::<Claims>(token, &decoding, &validation)
        .expect("verified token")
        .claims
}

fn bearer(headers: &BTreeMap<String, String>) -> String {
    headers["Authorization"]
        .strip_prefix("Bearer ")
        .expect("bearer prefix")
        .to_owned()
}

fn claim<'claims>(claims: &'claims Claims, name: &str) -> &'claims Json {
    claims.get(name).expect("claim")
}

#[tokio::test]
async fn a_call_type_that_is_not_an_mcp_call_returns_the_data_unchanged() {
    let signer = build_signer(&base_options());
    let auth = auth();
    let data = data(Some("weather"), None, Some(headers(&[("X-Trace", "abc")])));
    let mut request = input(&auth, &data, McpCallType::CallMcpTool);
    request.call_type = None;
    let output = sign(&signer, &request).await.expect("unsigned");
    assert_eq!(output.extra_headers, data.extra_headers);
    assert_eq!(output.tool_name, data.tool_name);
}

#[tokio::test]
async fn a_list_call_without_extra_headers_is_not_signed() {
    let signer = build_signer(&base_options());
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let output = sign(&signer, &input(&auth, &data, McpCallType::ListMcpTools))
        .await
        .expect("unsigned");
    assert_eq!(output.extra_headers, None);
}

#[tokio::test]
async fn a_list_call_with_empty_extra_headers_is_signed_and_clears_the_tool_name() {
    let signer = build_signer(&base_options());
    let auth = auth();
    let data = data(Some("weather"), None, Some(BTreeMap::new()));
    let output = sign(&signer, &input(&auth, &data, McpCallType::ListMcpTools))
        .await
        .expect("signed");
    assert_eq!(output.tool_name, None);
    let headers = output.extra_headers.expect("headers");
    let claims = decode_claims(&signer, &bearer(&headers));
    assert_eq!(
        claim(&claims, "scope"),
        &Json::Text("mcp:tools/list".to_owned())
    );
}

#[tokio::test]
async fn a_tool_call_gets_a_bearer_token_and_keeps_earlier_headers() {
    let signer = build_signer(&base_options());
    let auth = auth();
    let data = data(
        Some("weather"),
        None,
        Some(headers(&[("X-Trace", "abc"), ("X-Second", "one")])),
    );
    let output = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect("signed");
    let headers = output.extra_headers.expect("headers");
    assert_eq!(headers["X-Trace"], "abc");
    assert_eq!(headers["X-Second"], "one");
    let claims = decode_claims(&signer, &bearer(&headers));
    assert_eq!(
        claim(&claims, "scope"),
        &Json::Text("mcp:tools/call mcp:tools/weather:call".to_owned())
    );
    assert_eq!(claim(&claims, "iss"), &Json::Text("litellm".to_owned()));
    assert_eq!(claim(&claims, "aud"), &Json::Text(AUDIENCE.to_owned()));
    assert_eq!(
        claim(&claims, "sub"),
        &Json::Text("litellm-proxy".to_owned())
    );
    assert_eq!(
        claim(&claims, "exp"),
        &Json::Number(Number::from(NOW.saturating_add(300)))
    );
}

#[tokio::test]
async fn the_configured_issuer_audience_and_ttl_are_signed_into_the_token() {
    let mut options = base_options();
    options.issuer = Some(ISSUER.to_owned());
    options.audience = Some("custom-audience".to_owned());
    options.ttl_seconds = Some(60);
    let signer = build_signer(&options);
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let output = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect("signed");
    let claims = decode_claims(&signer, &bearer(&output.extra_headers.expect("headers")));
    assert_eq!(claim(&claims, "iss"), &Json::Text(ISSUER.to_owned()));
    assert_eq!(
        claim(&claims, "aud"),
        &Json::Text("custom-audience".to_owned())
    );
    assert_eq!(
        claim(&claims, "exp"),
        &Json::Number(Number::from(NOW.saturating_add(60)))
    );
}

#[tokio::test]
async fn a_channel_token_uses_its_own_audience_and_ttl() {
    let mut options = base_options();
    options.channel_token_audience = Some("channel".to_owned());
    options.channel_token_ttl = Some(60);
    let signer = build_signer(&options);
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let output = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect("signed");
    let headers = output.extra_headers.expect("headers");
    let channel = headers["x-mcp-channel-token"]
        .strip_prefix("Bearer ")
        .expect("bearer prefix");
    let entry = &get_jwks(&signer.key).keys[0];
    let decoding =
        DecodingKey::from_rsa_components(&entry.modulus, &entry.exponent).expect("decoding key");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.set_audience(&["channel"]);
    let claims = jsonwebtoken::decode::<Claims>(channel, &decoding, &validation)
        .expect("channel token")
        .claims;
    assert_eq!(claim(&claims, "aud"), &Json::Text("channel".to_owned()));
    assert_eq!(
        claim(&claims, "exp"),
        &Json::Number(Number::from(NOW.saturating_add(60)))
    );
}

#[tokio::test]
async fn there_is_no_channel_token_without_a_channel_audience() {
    let signer = build_signer(&base_options());
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let output = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect("signed");
    let headers = output.extra_headers.expect("headers");
    assert!(!headers.contains_key("x-mcp-channel-token"));
}

#[tokio::test]
async fn the_debug_header_reports_the_kid_when_enabled() {
    let mut options = base_options();
    options.debug_headers = true;
    let signer = build_signer(&options);
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let output = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect("signed");
    let headers = output.extra_headers.expect("headers");
    let debug = &headers["x-litellm-mcp-debug"];
    assert!(debug.starts_with(&format!("v=1; kid={};", signer.key.kid)));
    assert!(debug.contains("sub=litellm-proxy"));
    assert!(debug.contains("scope=mcp:tools/call mcp:tools/weather:call"));
}

#[tokio::test]
async fn there_is_no_debug_header_by_default() {
    let signer = build_signer(&base_options());
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let output = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect("signed");
    assert!(
        !output
            .extra_headers
            .expect("headers")
            .contains_key("x-litellm-mcp-debug")
    );
}

#[test]
fn the_debug_output_redacts_tokens() {
    let data = HookData {
        tool_name: Some("weather".to_owned()),
        incoming_bearer_token: Some(SecretString::from("opaque-incoming-token")),
        extra_headers: Some(headers(&[(
            "Authorization",
            "Bearer signed-outbound-token",
        )])),
    };
    let text = format!("{data:?}");
    assert!(!text.contains("signed-outbound-token"));
    assert!(!text.contains("opaque-incoming-token"));
    assert!(text.contains("Authorization"));
    assert!(text.contains("[redacted]"));
}

#[tokio::test]
async fn the_proxy_decoded_claims_are_used_when_no_token_is_verified() {
    let signer = build_signer(&base_options());
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let proxy_claims = Object::from([("sub".to_owned(), Json::Text("bob".to_owned()))]);
    let mut request = input(&auth, &data, McpCallType::CallMcpTool);
    request.jwt_claims = Some(&proxy_claims);
    let output = sign(&signer, &request).await.expect("signed");
    let claims = decode_claims(&signer, &bearer(&output.extra_headers.expect("headers")));
    assert_eq!(claim(&claims, "sub"), &Json::Text("bob".to_owned()));
}

#[tokio::test]
async fn verified_idp_claims_win_over_the_proxy_claims() {
    let key = idp_key();
    let (base, _server) = serve(r#"{"jwks_uri":"$BASE/jwks"}"#, &jwks_body(&key), "{}").await;
    let signer = build_signer(&with_discovery(&base));
    let auth = auth();
    let idp_claims = Object::from([
        ("sub".to_owned(), Json::Text("alice".to_owned())),
        (
            "exp".to_owned(),
            Json::Number(Number::from(
                i64::try_from(jsonwebtoken::get_current_timestamp())
                    .expect("clock")
                    .saturating_add(300),
            )),
        ),
    ]);
    let token = key.sign(&idp_claims).expect("idp token");
    let data = data(Some("weather"), Some(token.expose_secret()), None);
    let proxy_claims = Object::from([("sub".to_owned(), Json::Text("bob".to_owned()))]);
    let mut request = input(&auth, &data, McpCallType::CallMcpTool);
    request.jwt_claims = Some(&proxy_claims);
    let output = sign(&signer, &request).await.expect("signed");
    let claims = decode_claims(&signer, &bearer(&output.extra_headers.expect("headers")));
    assert_eq!(claim(&claims, "sub"), &Json::Text("alice".to_owned()));
}

#[tokio::test]
async fn a_failed_incoming_verification_is_a_401_with_the_signer_prefix() {
    let (base, _server) = serve(r#"{"jwks_uri":"$BASE/jwks"}"#, r#"{"keys":[]}"#, "{}").await;
    let signer = build_signer(&with_discovery(&base));
    let auth = auth();
    let data = data(Some("weather"), Some("a.b.c"), None);
    let error = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect_err("verification failure");
    assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
    assert!(matches!(error, SignError::Verification { .. }));
    let detail = error.detail();
    assert!(
        detail
            .error
            .starts_with("MCPJWTSigner: incoming token verification failed: ")
    );
}

#[tokio::test]
async fn an_inactive_introspection_answer_is_a_401() {
    let key = idp_key();
    let (base, _server) = serve(
        r#"{"jwks_uri":"$BASE/jwks"}"#,
        &jwks_body(&key),
        r#"{"active":false}"#,
    )
    .await;
    let mut options = with_discovery(&base);
    options.token_introspection_endpoint = Some(format!("{base}/introspect"));
    let signer = build_signer(&options);
    let auth = auth();
    let data = data(Some("weather"), Some("opaque-token"), None);
    let error = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect_err("inactive token");
    assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
    assert!(matches!(error, SignError::Introspection { .. }));
    assert!(
        error
            .detail()
            .error
            .ends_with("incoming token is inactive (introspection returned active=false)")
    );
}

#[tokio::test]
async fn an_opaque_token_is_introspected_when_an_endpoint_is_configured() {
    let key = idp_key();
    let (base, _server) = serve(
        r#"{"jwks_uri":"$BASE/jwks"}"#,
        &jwks_body(&key),
        r#"{"active":true,"sub":"carol"}"#,
    )
    .await;
    let mut options = with_discovery(&base);
    options.token_introspection_endpoint = Some(format!("{base}/introspect"));
    let signer = build_signer(&options);
    let auth = auth();
    let data = data(Some("weather"), Some("opaque-token"), None);
    let output = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect("signed");
    let claims = decode_claims(&signer, &bearer(&output.extra_headers.expect("headers")));
    assert_eq!(claim(&claims, "sub"), &Json::Text("carol".to_owned()));
}

#[tokio::test]
async fn an_opaque_token_without_an_endpoint_is_not_verified() {
    let (base, _server) = serve(r#"{"jwks_uri":"$BASE/jwks"}"#, r#"{"keys":[]}"#, "{}").await;
    let signer = build_signer(&with_discovery(&base));
    let auth = auth();
    let data = data(Some("weather"), Some("opaque-token"), None);
    let proxy_claims = Object::from([("sub".to_owned(), Json::Text("bob".to_owned()))]);
    let mut request = input(&auth, &data, McpCallType::CallMcpTool);
    request.jwt_claims = Some(&proxy_claims);
    let output = sign(&signer, &request).await.expect("signed");
    let claims = decode_claims(&signer, &bearer(&output.extra_headers.expect("headers")));
    assert_eq!(claim(&claims, "sub"), &Json::Text("bob".to_owned()));
}

#[tokio::test]
async fn missing_required_claims_are_a_403() {
    let mut options = base_options();
    options.required_claims = Some(vec!["email".to_owned()]);
    let signer = build_signer(&options);
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let error = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect_err("missing claims");
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert!(matches!(error, SignError::Claims(_)));
    assert!(error.detail().error.contains("required claims"));
}

#[tokio::test]
async fn the_signing_log_names_the_claims_and_never_the_token() {
    let signer = build_signer(&base_options());
    let auth = auth();
    let data = data(Some("weather"), None, None);
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let guard = set_default(subscriber);
    let output = sign(&signer, &input(&auth, &data, McpCallType::CallMcpTool))
        .await
        .expect("signed");
    drop(guard);
    let token = bearer(&output.extra_headers.expect("headers"));
    let text = String::from_utf8(sink.0.lock().expect("captured log").clone()).expect("utf8 log");
    assert!(text.contains("MCPJWTSigner: signed JWT"));
    assert!(text.contains("sub=litellm-proxy"));
    assert!(text.contains("call_type=call_mcp_tool"));
    assert!(!text.contains(&token));
}

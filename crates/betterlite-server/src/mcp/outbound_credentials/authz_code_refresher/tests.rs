#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::oauth_token_resolution::ports::TokenResponse;
use betterlite_db::oauth_token_resolution::{EnforceRequest, OauthBindingPort};
use betterlite_mcp::oauth_identity_binding::port_types::{
    BindingDenied, RefreshOwnership, RejectionCode,
};
use betterlite_mcp::server_record::MCPServer;
use chrono::DateTime;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::grant_body::{parse_expires_in, parse_scopes, read_grant_body};
use super::ports::{
    CredentialPersist, EpochClock, PersistRequest, ServerLookup, TokenEndpointPost,
};
use super::{AuthorizationCodeRefresher, refresh_form};
use crate::mcp::outbound_credentials::{OAuthToken, TokenRefresher, TokenStoreUnavailable};

type FakeRefresher = AuthorizationCodeRefresher<
    Arc<FakeLookup>,
    Arc<FakePost>,
    Arc<FakePersist>,
    Arc<FakeBinding>,
    FixedClock,
>;

struct Harness {
    refresher: FakeRefresher,
    post: Arc<FakePost>,
    persist: Arc<FakePersist>,
    binding: Arc<FakeBinding>,
}

struct FakeLookup {
    server: Option<Arc<MCPServer>>,
}

impl ServerLookup for FakeLookup {
    fn server(&self, _server_id: &str) -> Option<Arc<MCPServer>> {
        self.server.clone()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PostCall {
    url: String,
    form: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
}

struct FakePost {
    answer: Option<TokenResponse>,
    calls: Mutex<Vec<PostCall>>,
}

impl FakePost {
    fn calls(&self) -> Vec<PostCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl TokenEndpointPost for FakePost {
    async fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> Option<TokenResponse> {
        self.calls.lock().unwrap().push(PostCall {
            url: url.to_owned(),
            form: form.clone(),
            headers: headers.clone(),
        });
        self.answer.clone()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PersistRecord {
    user_id: String,
    server_id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scopes: Option<Vec<String>>,
    identity_binding_proof: Option<String>,
}

struct FakePersist {
    failure: Option<TokenStoreUnavailable>,
    calls: Mutex<Vec<PersistRecord>>,
}

impl FakePersist {
    fn calls(&self) -> Vec<PersistRecord> {
        self.calls.lock().unwrap().clone()
    }
}

impl CredentialPersist for FakePersist {
    async fn persist(&self, request: &PersistRequest<'_>) -> Result<(), TokenStoreUnavailable> {
        self.calls.lock().unwrap().push(PersistRecord {
            user_id: request.user_id.to_owned(),
            server_id: request.server_id.to_owned(),
            access_token: request.access_token.to_owned(),
            refresh_token: request.refresh_token.map(str::to_owned),
            expires_in: request.expires_in,
            scopes: request.scopes.map(<[String]>::to_vec),
            identity_binding_proof: request.identity_binding_proof.map(str::to_owned),
        });
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EnforceRecord {
    user_id: Option<String>,
    grant_type: String,
    ownership: Option<String>,
    response: TokenResponse,
}

struct FakeBinding {
    outcomes: Mutex<VecDeque<Result<Option<String>, BindingDenied>>>,
    calls: Mutex<Vec<EnforceRecord>>,
}

impl FakeBinding {
    fn calls(&self) -> Vec<EnforceRecord> {
        self.calls.lock().unwrap().clone()
    }
}

fn presented(ownership: Option<&RefreshOwnership>) -> Option<String> {
    match ownership? {
        RefreshOwnership::Presented(token) => Some(token.expose_secret().to_owned()),
        RefreshOwnership::Proven => None,
    }
}

impl OauthBindingPort for FakeBinding {
    type Server = MCPServer;
    type Ownership = RefreshOwnership;
    type Denied = BindingDenied;

    async fn enforce(
        &self,
        request: &EnforceRequest<'_, MCPServer, RefreshOwnership>,
    ) -> Result<Option<String>, BindingDenied> {
        self.calls.lock().unwrap().push(EnforceRecord {
            user_id: request.user_id.map(str::to_owned),
            grant_type: request.grant_type.to_owned(),
            ownership: presented(request.ownership),
            response: request.token_response.clone(),
        });
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(None))
    }

    async fn proof_matches(
        &self,
        _server: &MCPServer,
        _user_id: &str,
        _stored_proof: Option<&str>,
    ) -> bool {
        true
    }
}

struct FixedClock(f64);

impl EpochClock for FixedClock {
    fn now(&self) -> f64 {
        self.0
    }
}

fn oauth_server_making(
    token_url: Option<&str>,
    auth_method: Option<&str>,
    binding_mode: Option<&str>,
    client_secret: Option<&str>,
) -> Arc<MCPServer> {
    let binding = binding_mode
        .map(|mode| json!({"mode": mode, "issuer": "https://idp.test", "audiences": ["aud"]}));
    let value = json!({
        "server_id": "srv",
        "name": "Server",
        "transport": "http",
        "auth_type": "oauth2",
        "client_id": "client",
        "client_secret": client_secret,
        "token_url": token_url,
        "token_endpoint_auth_method": auth_method,
        "oauth_identity_binding": binding,
    });
    Arc::new(serde_json::from_value(value).unwrap())
}

fn oauth_server(token_url: Option<&str>, binding_mode: Option<&str>) -> Arc<MCPServer> {
    oauth_server_making(token_url, None, binding_mode, Some("secret"))
}

fn outage() -> BindingDenied {
    BindingDenied {
        code: RejectionCode::BindingFailed,
        description: "the upstream token does not belong to the caller".to_owned(),
        server_id: "srv".to_owned(),
    }
}

fn fixture(
    server: Option<Arc<MCPServer>>,
    answer: Option<TokenResponse>,
    outcomes: Vec<Result<Option<String>, BindingDenied>>,
) -> Harness {
    let post = Arc::new(FakePost {
        answer,
        calls: Mutex::new(Vec::new()),
    });
    let persist = Arc::new(FakePersist {
        failure: None,
        calls: Mutex::new(Vec::new()),
    });
    let binding = Arc::new(FakeBinding {
        outcomes: Mutex::new(VecDeque::from(outcomes)),
        calls: Mutex::new(Vec::new()),
    });
    let refresher = AuthorizationCodeRefresher {
        server_lookup: Arc::new(FakeLookup { server }),
        token_endpoint: Arc::clone(&post),
        persist: Arc::clone(&persist),
        binding: Arc::clone(&binding),
        clock: FixedClock(1_000.0),
    };
    Harness {
        refresher,
        post,
        persist,
        binding,
    }
}

fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

fn integer(value: i64) -> JsonValue {
    JsonValue::Number(serde_json::Number::from(value))
}

fn answer(fields: &[(&str, JsonValue)]) -> TokenResponse {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn grant_answer() -> TokenResponse {
    answer(&[
        ("access_token", text("fresh")),
        ("refresh_token", text("rotated")),
        ("expires_in", integer(3_600)),
        ("scope", text("read write")),
    ])
}

fn stored_token(refresh: Option<&str>, scopes: &[&str]) -> OAuthToken {
    OAuthToken {
        access_token: "stale".to_owned(),
        expires_at: None,
        refresh_token: refresh.map(str::to_owned),
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        identity_binding_proof: None,
    }
}

async fn refresh(
    harness: &Harness,
    token: &OAuthToken,
) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
    harness.refresher.refresh("user", "srv", token).await
}

#[tokio::test]
async fn a_token_without_a_refresh_token_returns_none_without_a_post() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(grant_answer()),
        Vec::new(),
    );
    let outcome = refresh(&harness, &stored_token(None, &[])).await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(harness.post.calls().len(), 0);
    assert_eq!(harness.binding.calls().len(), 0);
}

#[tokio::test]
async fn an_unknown_server_returns_none() {
    let harness = fixture(None, Some(grant_answer()), Vec::new());
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[])).await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(harness.post.calls().len(), 0);
}

#[tokio::test]
async fn a_server_without_a_token_url_returns_none() {
    for token_url in [None, Some("")] {
        let harness = fixture(
            Some(oauth_server(token_url, None)),
            Some(grant_answer()),
            Vec::new(),
        );
        let outcome = refresh(&harness, &stored_token(Some("stored"), &[])).await;
        assert_eq!(outcome.unwrap(), None);
        assert_eq!(harness.post.calls().len(), 0);
    }
}

#[tokio::test]
async fn a_misconfigured_client_auth_returns_none_before_the_binding_check() {
    let server = oauth_server_making(
        Some("https://idp.test/token"),
        Some("client_secret_basic"),
        Some("enforce"),
        None,
    );
    let harness = fixture(Some(server), Some(grant_answer()), vec![Ok(None), Ok(None)]);
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[])).await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(harness.post.calls().len(), 0);
    assert_eq!(harness.binding.calls().len(), 0);
}

#[tokio::test]
async fn an_enforced_binding_is_checked_before_the_post_with_an_empty_response() {
    let server = oauth_server(Some("https://idp.test/token"), Some("enforce"));
    let harness = fixture(Some(server), Some(grant_answer()), vec![Ok(None), Ok(None)]);
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
        .await
        .unwrap();
    assert!(outcome.is_some());
    let calls = harness.binding.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].response, TokenResponse::new());
    assert_eq!(calls[0].user_id.as_deref(), Some("user"));
    assert_eq!(calls[0].grant_type, "refresh_token");
    assert_eq!(calls[0].ownership.as_deref(), Some("stored"));
    assert_eq!(calls[1].response, grant_answer());
    assert_eq!(calls[1].ownership.as_deref(), Some("stored"));
}

#[tokio::test]
async fn a_precheck_denial_returns_none_without_a_post() {
    let server = oauth_server(Some("https://idp.test/token"), Some("enforce"));
    let harness = fixture(Some(server), Some(grant_answer()), vec![Err(outage())]);
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[])).await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(harness.post.calls().len(), 0);
    assert_eq!(harness.persist.calls().len(), 0);
}

#[tokio::test]
async fn the_form_carries_the_grant_and_the_client_auth_body() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(grant_answer()),
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[])).await;
    assert!(outcome.unwrap().is_some());
    let calls = harness.post.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, "https://idp.test/token");
    assert_eq!(
        calls[0].form,
        BTreeMap::from([
            ("client_id".to_owned(), "client".to_owned()),
            ("client_secret".to_owned(), "secret".to_owned()),
            ("grant_type".to_owned(), "refresh_token".to_owned()),
            ("refresh_token".to_owned(), "stored".to_owned()),
        ])
    );
    assert_eq!(calls[0].headers, BTreeMap::new());
}

#[test]
fn a_client_auth_body_key_wins_over_the_grant_keys() {
    let body = BTreeMap::from([(
        "grant_type".to_owned(),
        SecretString::from("client_credentials".to_owned()),
    )]);
    let form = refresh_form("stored", &body);
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    assert_eq!(
        form.get("refresh_token").map(String::as_str),
        Some("stored")
    );
}

#[tokio::test]
async fn a_missing_answer_returns_none_and_persists_nothing() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        None,
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[])).await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(harness.persist.calls().len(), 0);
}

#[tokio::test]
async fn a_missing_or_empty_access_token_returns_none_and_persists_nothing() {
    let bodies = [
        answer(&[("refresh_token", text("rotated"))]),
        answer(&[("access_token", text(""))]),
        answer(&[("access_token", integer(7))]),
    ];
    for body in bodies {
        let harness = fixture(
            Some(oauth_server(Some("https://idp.test/token"), None)),
            Some(body),
            vec![Ok(None)],
        );
        let outcome = refresh(&harness, &stored_token(Some("stored"), &[])).await;
        assert_eq!(outcome.unwrap(), None);
        assert_eq!(harness.persist.calls().len(), 0);
    }
}

#[tokio::test]
async fn the_second_check_runs_for_an_unbound_server_with_the_real_body() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(grant_answer()),
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.access_token, "fresh");
    let calls = harness.binding.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].response, grant_answer());
    assert_eq!(calls[0].user_id.as_deref(), Some("user"));
}

#[tokio::test]
async fn a_second_check_denial_returns_none_and_persists_nothing() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(grant_answer()),
        vec![Err(outage())],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[])).await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(harness.persist.calls().len(), 0);
}

#[tokio::test]
async fn an_audit_binding_is_checked_once_and_persists_without_a_proof() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), Some("audit"))),
        Some(grant_answer()),
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(harness.binding.calls().len(), 1);
    assert_eq!(outcome.identity_binding_proof, None);
    assert_eq!(harness.persist.calls()[0].identity_binding_proof, None);
}

#[tokio::test]
async fn a_proof_is_persisted_and_returned_on_the_token() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(grant_answer()),
        vec![Ok(Some("bound".to_owned()))],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.identity_binding_proof.as_deref(), Some("bound"));
    assert_eq!(
        harness.persist.calls()[0].identity_binding_proof.as_deref(),
        Some("bound")
    );
}

#[tokio::test]
async fn a_rotated_refresh_token_replaces_the_old_one() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(grant_answer()),
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.refresh_token.as_deref(), Some("rotated"));
    assert_eq!(
        harness.persist.calls()[0].refresh_token.as_deref(),
        Some("rotated")
    );
}

#[tokio::test]
async fn an_omitted_refresh_token_is_carried_forward() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(answer(&[("access_token", text("fresh"))])),
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.refresh_token.as_deref(), Some("stored"));
    assert_eq!(
        harness.persist.calls()[0].refresh_token.as_deref(),
        Some("stored")
    );
}

#[tokio::test]
async fn an_omitted_scope_keeps_the_stored_scopes() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(answer(&[("access_token", text("fresh"))])),
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &["read"]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.scopes, vec!["read".to_owned()]);
    assert_eq!(
        harness.persist.calls()[0].scopes,
        Some(vec!["read".to_owned()])
    );
}

#[tokio::test]
async fn empty_scopes_persist_as_none_and_return_empty() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(answer(&[("access_token", text("fresh"))])),
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.scopes, Vec::<String>::new());
    assert_eq!(harness.persist.calls()[0].scopes, None);
}

#[tokio::test]
async fn expires_at_is_the_clock_plus_expires_in() {
    let harness = fixture(
        Some(oauth_server(Some("https://idp.test/token"), None)),
        Some(grant_answer()),
        vec![Ok(None)],
    );
    let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.expires_at, DateTime::from_timestamp(4_600, 0));
    assert_eq!(harness.persist.calls()[0].expires_in, Some(3_600));
}

#[tokio::test]
async fn a_zero_or_negative_expires_in_is_used_as_is() {
    for (expires_in, expected) in [(0_i64, 1_000_i64), (-120, 880)] {
        let harness = fixture(
            Some(oauth_server(Some("https://idp.test/token"), None)),
            Some(answer(&[
                ("access_token", text("fresh")),
                ("expires_in", integer(expires_in)),
            ])),
            vec![Ok(None)],
        );
        let outcome = refresh(&harness, &stored_token(Some("stored"), &[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.expires_at, DateTime::from_timestamp(expected, 0));
        assert_eq!(harness.persist.calls()[0].expires_in, Some(expires_in));
    }
}

#[tokio::test]
async fn a_persist_failure_is_returned_to_the_caller() {
    let post = Arc::new(FakePost {
        answer: Some(grant_answer()),
        calls: Mutex::new(Vec::new()),
    });
    let persist = Arc::new(FakePersist {
        failure: Some(TokenStoreUnavailable {
            detail: "database not connected".to_owned(),
        }),
        calls: Mutex::new(Vec::new()),
    });
    let binding = Arc::new(FakeBinding {
        outcomes: Mutex::new(VecDeque::from(vec![Ok(None)])),
        calls: Mutex::new(Vec::new()),
    });
    let refresher = AuthorizationCodeRefresher {
        server_lookup: Arc::new(FakeLookup {
            server: Some(oauth_server(Some("https://idp.test/token"), None)),
        }),
        token_endpoint: Arc::clone(&post),
        persist: Arc::clone(&persist),
        binding: Arc::clone(&binding),
        clock: FixedClock(1_000.0),
    };
    let outcome = refresher
        .refresh("user", "srv", &stored_token(Some("stored"), &[]))
        .await;
    assert_eq!(
        outcome.unwrap_err(),
        TokenStoreUnavailable {
            detail: "database not connected".to_owned(),
        }
    );
    assert_eq!(persist.calls().len(), 1);
}

#[test]
fn parse_expires_in_follows_python_int_rules() {
    assert_eq!(parse_expires_in(&JsonValue::Bool(true)), None);
    assert_eq!(parse_expires_in(&JsonValue::Bool(false)), None);
    assert_eq!(parse_expires_in(&JsonValue::Null), None);
    assert_eq!(parse_expires_in(&JsonValue::Array(Vec::new())), None);
    assert_eq!(parse_expires_in(&JsonValue::Object(BTreeMap::new())), None);
    assert_eq!(parse_expires_in(&integer(3_600)), Some(3_600));
    assert_eq!(
        parse_expires_in(&JsonValue::Number(
            serde_json::Number::from_f64(3_600.0).unwrap()
        )),
        None
    );
    assert_eq!(
        parse_expires_in(&JsonValue::Number(serde_json::Number::from(u64::MAX))),
        None
    );
    assert_eq!(parse_expires_in(&text("3600")), Some(3_600));
    assert_eq!(parse_expires_in(&text(" +1_0 ")), Some(10));
    assert_eq!(parse_expires_in(&text("-7")), Some(-7));
    assert_eq!(parse_expires_in(&text("0")), Some(0));
    assert_eq!(parse_expires_in(&text("3600.0")), None);
    assert_eq!(parse_expires_in(&text("")), None);
    assert_eq!(parse_expires_in(&text("1__0")), None);
    assert_eq!(parse_expires_in(&text("_1")), None);
    assert_eq!(parse_expires_in(&text("1_")), None);
    assert_eq!(parse_expires_in(&text("  ")), None);
}

#[test]
fn parse_scopes_splits_a_non_empty_string() {
    assert_eq!(
        parse_scopes(&text("read write")),
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
    assert_eq!(
        parse_scopes(&text("  read\twrite  ")),
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
    assert_eq!(parse_scopes(&text("   ")), Some(Vec::new()));
    assert_eq!(parse_scopes(&text("")), None);
    assert_eq!(parse_scopes(&integer(7)), None);
    assert_eq!(parse_scopes(&JsonValue::Null), None);
}

#[test]
fn read_grant_body_reads_the_grant_fields() {
    let body = answer(&[
        ("access_token", text("fresh")),
        ("refresh_token", text("")),
        ("expires_in", text("60")),
        ("scope", text("read write")),
    ]);
    let grant = read_grant_body(&body).unwrap();
    assert_eq!(grant.access_token, "fresh");
    assert_eq!(grant.rotated_refresh, None);
    assert_eq!(grant.expires_in, Some(60));
    assert_eq!(
        grant.scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
    assert!(read_grant_body(&answer(&[("refresh_token", text("x"))])).is_none());
}

#[test]
fn replays_the_refresh_grant_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/refresh_grant_body");
    for entry in std::fs::read_dir(corpus).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        if let Ok(JsonValue::Object(body)) = serde_json::from_slice::<JsonValue>(&bytes) {
            drop(read_grant_body(&body));
        }
    }
}

#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTokenEndpointAuthMethod};
use betterlite_mcp::server_record::{MCPServer, OAuth2Flow, SecretCredential};
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use super::auth::{McpAuthOverride, McpAuthValue};
use super::error::OAuthCacheError;
use super::per_user::PerUserTokenBackend;
use super::{
    MCPOAuth2TokenCache, MCPPerUserTokenCache, compute_per_user_token_ttl, resolve_mcp_auth,
    resolved_token_header,
};
use crate::mcp::outbound_credentials::WallClock;
use crate::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use crate::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome,
};

fn base_server() -> MCPServer {
    serde_json::from_str(r#"{"server_id":"srv-1","name":"Server","transport":"http"}"#)
        .expect("the base server decodes")
}

fn secret_credential(text: &str) -> SecretCredential {
    SecretCredential(StoredSecret(SecretString::from(text.to_owned())))
}

fn client_server() -> MCPServer {
    let mut server = base_server();
    server.fields.auth_type = Some(MCPAuth::Oauth2);
    server.fields.oauth2_flow = Some(OAuth2Flow::ClientCredentials);
    server.fields.client_id = Some("client".to_owned());
    server.fields.client_secret = Some(secret_credential("secret"));
    server.fields.token_url = Some("https://idp.example.com/token".to_owned());
    server.fields.scopes = Some(vec!["read".to_owned(), "write".to_owned()]);
    server
}

struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(
                DateTime::from_timestamp_millis(1_000_000).expect("the epoch is representable"),
            ),
        }
    }

    fn advance(&self, seconds: i64) {
        let delta = TimeDelta::try_seconds(seconds).expect("the delta is representable");
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now
            .checked_add_signed(delta)
            .expect("the clock stays representable");
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

type RecordedCall = (String, Vec<(String, String)>, Vec<(String, String)>);

struct ScriptedPost {
    outcomes: Mutex<VecDeque<TokenEndpointOutcome>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedPost {
    fn new(outcomes: Vec<TokenEndpointOutcome>) -> Self {
        Self {
            outcomes: Mutex::new(VecDeque::from(outcomes)),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn form_pairs(form: &[(String, SecretString)]) -> Vec<(String, String)> {
    form.iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

#[async_trait]
impl M2mTokenEndpointPost for ScriptedPost {
    async fn post(
        &self,
        url: &str,
        form: &[(String, SecretString)],
        headers: &[(String, SecretString)],
    ) -> TokenEndpointOutcome {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_owned(), form_pairs(form), form_pairs(headers)));
        let mut outcomes = self.outcomes.lock().unwrap_or_else(PoisonError::into_inner);
        outcomes
            .pop_front()
            .unwrap_or(TokenEndpointOutcome::Unreachable {
                detail: "the test scripted no outcome".to_owned(),
            })
    }
}

fn success(body: &str) -> TokenEndpointOutcome {
    TokenEndpointOutcome::Success {
        body: serde_json::from_str(body).expect("the scripted body is a token response"),
    }
}

fn cache_with(post: &Arc<ScriptedPost>, clock: &Arc<ManualClock>) -> MCPOAuth2TokenCache {
    MCPOAuth2TokenCache::new(Arc::clone(post), Arc::clone(clock))
}

#[test]
fn token_identity_hashes_the_material_under_the_server_prefix() {
    let base = MCPOAuth2TokenCache::token_identity(&client_server());
    assert!(base.starts_with("srv-1:"));
    assert_eq!(base.len(), "srv-1:".len() + 64);
    assert_eq!(base, MCPOAuth2TokenCache::token_identity(&client_server()));
    let mut configured = client_server();
    configured.fields.token_url = None;
    configured.fields.configured_token_url = Some("https://idp.example.com/token".to_owned());
    assert_eq!(MCPOAuth2TokenCache::token_identity(&configured), base);
}

#[test]
fn token_identity_changes_with_any_request_material() {
    let base = MCPOAuth2TokenCache::token_identity(&client_server());
    let mut changed_client = client_server();
    changed_client.fields.client_id = Some("other".to_owned());
    let mut rotated_secret = client_server();
    rotated_secret.fields.client_secret = Some(secret_credential("rotated"));
    let mut retargeted = client_server();
    retargeted.fields.token_url = Some("https://other.example.com/token".to_owned());
    let mut narrowed = client_server();
    narrowed.fields.scopes = Some(vec!["read".to_owned()]);
    let mut resourced = client_server();
    resourced.fields.upstream_resource = Some("https://api.example.com".to_owned());
    let mut basic = client_server();
    basic.fields.token_endpoint_auth_method = Some(MCPTokenEndpointAuthMethod::ClientSecretBasic);
    let mut automatic = client_server();
    automatic.fields.url = Some("https://mcp.example.com/sse".to_owned());
    automatic.fields.upstream_resource = Some("auto".to_owned());
    for server in [
        changed_client,
        rotated_secret,
        retargeted,
        narrowed,
        resourced,
        basic,
        automatic,
    ] {
        assert_ne!(MCPOAuth2TokenCache::token_identity(&server), base);
    }
}

#[tokio::test]
async fn async_get_token_serves_the_cached_token_until_the_ttl_lapses() {
    let post = Arc::new(ScriptedPost::new(vec![
        success(r#"{"access_token":"tok-0","expires_in":3600}"#),
        success(r#"{"access_token":"tok-1","expires_in":3600}"#),
    ]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let server = client_server();
    let first = cache
        .async_get_token(&server)
        .await
        .expect("the first fetch succeeds");
    assert_eq!(first, Some("tok-0".to_owned()));
    assert_eq!(post.calls().len(), 1);
    clock.advance(3539);
    let cached = cache
        .async_get_token(&server)
        .await
        .expect("the cached read succeeds");
    assert_eq!(cached, Some("tok-0".to_owned()));
    assert_eq!(post.calls().len(), 1);
    clock.advance(1);
    let refetched = cache
        .async_get_token(&server)
        .await
        .expect("the second fetch succeeds");
    assert_eq!(refetched, Some("tok-1".to_owned()));
    assert_eq!(post.calls().len(), 2);
}

#[tokio::test]
async fn async_get_token_returns_none_without_client_credentials_config() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut missing_flow = client_server();
    missing_flow.fields.oauth2_flow = None;
    let mut authorization_code = client_server();
    authorization_code.fields.oauth2_flow = Some(OAuth2Flow::AuthorizationCode);
    let mut missing_secret = client_server();
    missing_secret.fields.client_secret = None;
    let mut empty_secret = client_server();
    empty_secret.fields.client_secret = Some(secret_credential(""));
    let mut empty_client = client_server();
    empty_client.fields.client_id = Some(String::new());
    let mut missing_url = client_server();
    missing_url.fields.token_url = None;
    for server in [
        missing_flow,
        authorization_code,
        missing_secret,
        empty_secret,
        empty_client,
        missing_url,
    ] {
        assert_eq!(
            cache.async_get_token(&server).await.expect("no fetch runs"),
            None
        );
    }
    assert!(post.calls().is_empty());
}

#[tokio::test]
async fn async_get_token_reads_the_configured_token_url() {
    let post = Arc::new(ScriptedPost::new(vec![success(
        r#"{"access_token":"tok"}"#,
    )]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut configured_only = client_server();
    configured_only.fields.token_url = Some(String::new());
    configured_only.fields.configured_token_url =
        Some("https://configured.example.com/token".to_owned());
    assert_eq!(
        cache
            .async_get_token(&configured_only)
            .await
            .expect("the fetch succeeds"),
        Some("tok".to_owned())
    );
    assert_eq!(
        post.calls()
            .first()
            .map(|(url, _form, _headers)| url.as_str()),
        Some("https://configured.example.com/token")
    );
}

#[tokio::test]
async fn async_get_token_fetches_once_for_concurrent_callers() {
    let post = Arc::new(ScriptedPost::new(vec![success(
        r#"{"access_token":"tok","expires_in":600}"#,
    )]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let server = client_server();
    let results =
        futures_util::future::join_all((0..8).map(|_index| cache.async_get_token(&server))).await;
    assert_eq!(post.calls().len(), 1);
    for result in results {
        assert_eq!(
            result.expect("every caller gets a token"),
            Some("tok".to_owned())
        );
    }
}

#[tokio::test]
async fn fetch_token_posts_client_secret_post_by_default() {
    let post = Arc::new(ScriptedPost::new(vec![success(
        r#"{"access_token":"tok","expires_in":3600}"#,
    )]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut server = client_server();
    server.fields.upstream_resource = Some("https://api.example.com".to_owned());
    let (token, ttl) = cache
        .fetch_token(&server)
        .await
        .expect("the fetch succeeds");
    assert_eq!(token, "tok");
    assert_eq!(ttl, 3540);
    let calls = post.calls();
    let (url, form, headers) = calls.first().expect("one post");
    assert_eq!(url, "https://idp.example.com/token");
    assert_eq!(
        form,
        &vec![
            ("grant_type".to_owned(), "client_credentials".to_owned()),
            ("client_id".to_owned(), "client".to_owned()),
            ("client_secret".to_owned(), "secret".to_owned()),
            ("resource".to_owned(), "https://api.example.com".to_owned()),
            ("scope".to_owned(), "read write".to_owned()),
        ]
    );
    assert!(headers.is_empty());
}

#[tokio::test]
async fn fetch_token_moves_the_client_secret_into_the_basic_header() {
    let post = Arc::new(ScriptedPost::new(vec![success(
        r#"{"access_token":"tok","expires_in":3600}"#,
    )]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut basic = client_server();
    basic.fields.token_endpoint_auth_method = Some(MCPTokenEndpointAuthMethod::ClientSecretBasic);
    cache
        .fetch_token(&basic)
        .await
        .expect("the basic fetch succeeds");
    let calls = post.calls();
    let (_url, form, headers) = calls.first().expect("one post");
    assert_eq!(
        form,
        &vec![
            ("grant_type".to_owned(), "client_credentials".to_owned()),
            ("scope".to_owned(), "read write".to_owned()),
        ]
    );
    assert_eq!(headers.len(), 1);
    let (name, value) = headers.first().expect("one header");
    assert_eq!(name, "Authorization");
    assert!(value.starts_with("Basic "));
}

#[tokio::test]
async fn fetch_token_omits_an_empty_scope() {
    let post = Arc::new(ScriptedPost::new(vec![success(
        r#"{"access_token":"tok","expires_in":3600}"#,
    )]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut unscoped = client_server();
    unscoped.fields.scopes = Some(Vec::new());
    cache
        .fetch_token(&unscoped)
        .await
        .expect("the fetch succeeds");
    let calls = post.calls();
    let (_url, form, _headers) = calls.first().expect("one post");
    assert_eq!(
        form.last().map(|(name, _value)| name.as_str()),
        Some("client_secret")
    );
}

#[tokio::test]
async fn fetch_token_buffers_and_parses_expires_in() {
    let cases = [
        (r#"{"access_token":"tok"}"#, 3540),
        (r#"{"access_token":"tok","expires_in":null}"#, 3540),
        (r#"{"access_token":"tok","expires_in":"120"}"#, 60),
        (r#"{"access_token":"tok","expires_in":" 36 "}"#, 10),
        (r#"{"access_token":"tok","expires_in":"soon"}"#, 3540),
        (r#"{"access_token":"tok","expires_in":1.5}"#, 10),
        (r#"{"access_token":"tok","expires_in":true}"#, 10),
        (r#"{"access_token":"tok","expires_in":30}"#, 10),
        (r#"{"access_token":"tok","expires_in":10}"#, 10),
        (r#"{"access_token":"tok","expires_in":70}"#, 10),
        (r#"{"access_token":"tok","expires_in":71}"#, 11),
    ];
    for (body, ttl) in cases {
        let post = Arc::new(ScriptedPost::new(vec![success(body)]));
        let clock = Arc::new(ManualClock::new());
        let cache = cache_with(&post, &clock);
        let fetched = cache
            .fetch_token(&client_server())
            .await
            .expect("the fetch succeeds");
        assert_eq!(fetched.1, ttl, "for {body}");
    }
}

#[tokio::test]
async fn fetch_token_names_an_unusable_access_token() {
    let post = Arc::new(ScriptedPost::new(vec![
        success(r#"{"expires_in":3600}"#),
        success(r#"{"access_token":"","expires_in":3600}"#),
        success(r#"{"access_token":42,"expires_in":3600}"#),
    ]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let server = client_server();
    for _index in 0..3 {
        let error = cache
            .fetch_token(&server)
            .await
            .expect_err("the response has no usable access token");
        assert!(matches!(error, OAuthCacheError::MissingAccessToken { .. }));
    }
}

#[tokio::test]
async fn fetch_token_names_a_denied_or_unreachable_endpoint() {
    let post = Arc::new(ScriptedPost::new(vec![
        TokenEndpointOutcome::Denied {
            status_code: 401,
            detail: "token endpoint returned HTTP 401".to_owned(),
        },
        TokenEndpointOutcome::Unreachable {
            detail: "connect failed".to_owned(),
        },
    ]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let server = client_server();
    let denied = cache
        .fetch_token(&server)
        .await
        .expect_err("the endpoint denials the request");
    assert!(matches!(
        denied,
        OAuthCacheError::Denied {
            status_code: 401,
            ..
        }
    ));
    let unreachable = cache
        .fetch_token(&server)
        .await
        .expect_err("the endpoint is unreachable");
    assert!(matches!(unreachable, OAuthCacheError::Unreachable { .. }));
}

#[tokio::test]
async fn fetch_token_names_the_missing_fields() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut incomplete = client_server();
    incomplete.fields.client_secret = None;
    incomplete.fields.token_url = None;
    let error = cache
        .fetch_token(&incomplete)
        .await
        .expect_err("the config is incomplete");
    assert!(matches!(
        error,
        OAuthCacheError::MissingFields {
            client_secret: false,
            token_url: false,
            ..
        }
    ));
}

#[tokio::test]
async fn invalidate_removes_every_identity_of_the_server() {
    let post = Arc::new(ScriptedPost::new(vec![
        success(r#"{"access_token":"a"}"#),
        success(r#"{"access_token":"b"}"#),
        success(r#"{"access_token":"c"}"#),
        success(r#"{"access_token":"d"}"#),
    ]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let first = client_server();
    let mut second = client_server();
    second.fields.scopes = Some(vec!["read".to_owned()]);
    for server in [&first, &second] {
        cache
            .async_get_token(server)
            .await
            .expect("the fetch succeeds");
    }
    assert_eq!(post.calls().len(), 2);
    cache.invalidate("srv-1");
    assert_eq!(
        cache
            .async_get_token(&first)
            .await
            .expect("the refetch succeeds"),
        Some("c".to_owned())
    );
    assert_eq!(
        cache
            .async_get_token(&second)
            .await
            .expect("the refetch succeeds"),
        Some("d".to_owned())
    );
    assert_eq!(post.calls().len(), 4);
}

#[tokio::test]
async fn invalidate_leaves_other_servers_cached() {
    let post = Arc::new(ScriptedPost::new(vec![
        success(r#"{"access_token":"a"}"#),
        success(r#"{"access_token":"b"}"#),
    ]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let first = client_server();
    let mut other = client_server();
    other.fields.server_id = "srv".to_owned();
    cache
        .async_get_token(&first)
        .await
        .expect("the fetch succeeds");
    cache
        .async_get_token(&other)
        .await
        .expect("the fetch succeeds");
    cache.invalidate("srv-1");
    assert_eq!(
        cache
            .async_get_token(&other)
            .await
            .expect("the cached read succeeds"),
        Some("b".to_owned())
    );
    assert_eq!(post.calls().len(), 2);
}

#[test]
fn the_cache_evicts_the_oldest_entry_at_the_max_size() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    for index in 0..super::OAUTH2_TOKEN_CACHE_MAX_SIZE {
        cache.insert(format!("srv-1:{index}"), "token".to_owned(), 600);
    }
    assert_eq!(cache.cached("srv-1:0"), Some("token".to_owned()));
    cache.insert("srv-1:new".to_owned(), "fresh".to_owned(), 600);
    assert_eq!(cache.cached("srv-1:0"), None);
    assert_eq!(cache.cached("srv-1:new"), Some("fresh".to_owned()));
    assert_eq!(cache.cached("srv-1:1"), Some("token".to_owned()));
}

struct FakeError;

impl std::fmt::Debug for FakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FakeError")
    }
}

impl std::fmt::Display for FakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the fake backend failed")
    }
}

impl std::error::Error for FakeError {}

struct FakeBackend {
    entries: Mutex<BTreeMap<String, String>>,
    fail_get: bool,
    fail_set: bool,
    fail_delete: bool,
    set_calls: Mutex<Vec<(String, i64)>>,
    delete_calls: Mutex<Vec<String>>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            fail_get: false,
            fail_set: false,
            fail_delete: false,
            set_calls: Mutex::new(Vec::new()),
            delete_calls: Mutex::new(Vec::new()),
        }
    }

    fn with_entry(key: &str, value: &str) -> Self {
        let backend = Self::new();
        backend
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.to_owned());
        backend
    }

    fn failing_set() -> Self {
        Self {
            fail_set: true,
            ..Self::new()
        }
    }

    fn failing_delete() -> Self {
        Self {
            fail_delete: true,
            ..Self::new()
        }
    }

    fn failing_get() -> Self {
        Self {
            fail_get: true,
            ..Self::new()
        }
    }

    fn value(&self, key: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }
}

#[async_trait]
impl PerUserTokenBackend for FakeBackend {
    type Error = FakeError;

    async fn get(&self, key: &str) -> Result<Option<String>, FakeError> {
        if self.fail_get {
            return Err(FakeError);
        }
        Ok(self.value(key))
    }

    async fn set(&self, key: &str, value: &str, ttl: i64) -> Result<(), FakeError> {
        if self.fail_set {
            return Err(FakeError);
        }
        self.set_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), ttl));
        drop(
            self.entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key.to_owned(), value.to_owned()),
        );
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), FakeError> {
        if self.fail_delete {
            return Err(FakeError);
        }
        self.delete_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        drop(
            self.entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(key),
        );
        Ok(())
    }
}

fn codec() -> OAuthTokenCacheCodec {
    OAuthTokenCacheCodec {
        encrypt: Box::new(|plain: &str| format!("sealed:{plain}")),
        decrypt: Box::new(|blob: &str| blob.strip_prefix("sealed:").map(str::to_owned)),
    }
}

#[test]
fn per_user_cache_key_matches_litellm() {
    assert_eq!(
        MCPPerUserTokenCache::<FakeBackend>::cache_key("user-1", "srv-1"),
        "mcp:per_user_token:user-1:srv-1"
    );
}

#[tokio::test]
async fn per_user_set_stores_an_encrypted_value_with_the_ttl_and_proof() {
    let cache = MCPPerUserTokenCache::new(FakeBackend::new(), codec());
    cache
        .set("user-1", "srv-1", "tok", 120, Some("proof"))
        .await;
    let stored = cache
        .backend
        .value("mcp:per_user_token:user-1:srv-1")
        .expect("the value is stored");
    assert_ne!(stored, "tok");
    assert_ne!(stored, "sealed:tok");
    assert!(stored.starts_with("sealed:litellm-bound-oauth-v1:"));
    assert_eq!(
        cache
            .backend
            .set_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_slice(),
        &[("mcp:per_user_token:user-1:srv-1".to_owned(), 120)]
    );
    let token = cache
        .get_token("user-1", "srv-1")
        .await
        .expect("the token decodes");
    assert_eq!(token.access_token, "tok");
    assert_eq!(token.identity_binding_proof.as_deref(), Some("proof"));
    assert_eq!(cache.get("user-1", "srv-1").await, Some("tok".to_owned()));
}

#[tokio::test]
async fn per_user_set_stores_a_bearer_only_value_without_a_proof() {
    let cache = MCPPerUserTokenCache::new(FakeBackend::new(), codec());
    cache.set("user-1", "srv-1", "tok", 60, None).await;
    let stored = cache
        .backend
        .value("mcp:per_user_token:user-1:srv-1")
        .expect("the value is stored");
    assert_eq!(stored, "sealed:tok");
    assert_eq!(cache.get("user-1", "srv-1").await, Some("tok".to_owned()));
}

#[tokio::test]
async fn per_user_set_swallows_a_backend_failure() {
    let cache = MCPPerUserTokenCache::new(FakeBackend::failing_set(), codec());
    cache.set("user-1", "srv-1", "tok", 60, None).await;
    assert_eq!(cache.get("user-1", "srv-1").await, None);
}

#[tokio::test]
async fn per_user_get_token_reads_an_undecryptable_value_as_a_miss() {
    let cache = MCPPerUserTokenCache::new(
        FakeBackend::with_entry("mcp:per_user_token:user-1:srv-1", "not-a-sealed-value"),
        codec(),
    );
    assert_eq!(cache.get_token("user-1", "srv-1").await, None);
    assert_eq!(cache.get("user-1", "srv-1").await, None);
}

#[tokio::test]
async fn per_user_get_token_swallows_a_backend_failure() {
    let cache = MCPPerUserTokenCache::new(FakeBackend::failing_get(), codec());
    assert_eq!(cache.get_token("user-1", "srv-1").await, None);
}

#[tokio::test]
async fn per_user_delete_evicts_the_key_and_swallows_failures() {
    let cache = MCPPerUserTokenCache::new(FakeBackend::new(), codec());
    cache.set("user-1", "srv-1", "tok", 60, None).await;
    cache.delete("user-1", "srv-1").await;
    assert_eq!(cache.get("user-1", "srv-1").await, None);
    assert_eq!(
        cache
            .backend
            .delete_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_slice(),
        &["mcp:per_user_token:user-1:srv-1".to_owned()]
    );
    let failing = MCPPerUserTokenCache::new(FakeBackend::failing_delete(), codec());
    failing.delete("user-1", "srv-1").await;
}

#[test]
fn compute_per_user_token_ttl_uses_the_configured_value_when_shorter() {
    let mut configured = base_server();
    configured.fields.token_storage_ttl_seconds = Some(7200);
    assert_eq!(compute_per_user_token_ttl(&configured, Some(86_400)), 7200);
    assert_eq!(compute_per_user_token_ttl(&configured, None), 7200);
    let mut zero = base_server();
    zero.fields.token_storage_ttl_seconds = Some(0);
    assert_eq!(compute_per_user_token_ttl(&zero, None), 1);
}

#[test]
fn compute_per_user_token_ttl_is_bounded_by_the_token_lifetime() {
    let mut configured = base_server();
    configured.fields.token_storage_ttl_seconds = Some(7200);
    assert_eq!(compute_per_user_token_ttl(&configured, Some(1800)), 1740);
    let unconfigured = base_server();
    assert_eq!(compute_per_user_token_ttl(&unconfigured, Some(1800)), 1740);
    assert_eq!(compute_per_user_token_ttl(&unconfigured, Some(30)), 1);
    assert_eq!(compute_per_user_token_ttl(&unconfigured, Some(61)), 1);
    assert_eq!(compute_per_user_token_ttl(&unconfigured, Some(62)), 2);
}

#[test]
fn compute_per_user_token_ttl_falls_back_to_the_default() {
    assert_eq!(compute_per_user_token_ttl(&base_server(), None), 43_200);
}

#[tokio::test]
async fn resolve_mcp_auth_prefers_the_request_header() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut server = client_server();
    server.fields.authentication_token = Some(secret_credential("static"));
    let text = McpAuthOverride::Text(SecretString::from("caller".to_owned()));
    let resolved = resolve_mcp_auth(&server, Some(&text), &cache)
        .await
        .expect("the override resolves");
    match resolved {
        Some(McpAuthValue::Text(value)) => assert_eq!(value.expose_secret(), "caller"),
        Some(McpAuthValue::Headers(_)) | None => panic!("the text override is returned as text"),
    }
    assert!(post.calls().is_empty());
}

#[tokio::test]
async fn resolve_mcp_auth_returns_a_header_map_override() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let server = client_server();
    let mut headers = BTreeMap::new();
    headers.insert(
        "x-api-key".to_owned(),
        SecretString::from("caller-key".to_owned()),
    );
    let mapped = McpAuthOverride::Headers(headers);
    let resolved = resolve_mcp_auth(&server, Some(&mapped), &cache)
        .await
        .expect("the header override resolves");
    match resolved {
        Some(McpAuthValue::Headers(values)) => {
            assert_eq!(
                values
                    .get("x-api-key")
                    .map(|value| value.expose_secret().as_str()),
                Some("caller-key")
            );
        }
        Some(McpAuthValue::Text(_)) | None => panic!("the header override is returned as headers"),
    }
    assert!(post.calls().is_empty());
}

#[tokio::test]
async fn resolve_mcp_auth_fetches_a_client_credentials_token() {
    let post = Arc::new(ScriptedPost::new(vec![success(
        r#"{"access_token":"minted"}"#,
    )]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut server = client_server();
    server.fields.authentication_token = Some(secret_credential("static"));
    let resolved = resolve_mcp_auth(&server, None, &cache)
        .await
        .expect("the token resolves");
    match resolved {
        Some(McpAuthValue::Text(value)) => assert_eq!(value.expose_secret(), "minted"),
        Some(McpAuthValue::Headers(_)) | None => panic!("a minted token is a text value"),
    }
    assert_eq!(post.calls().len(), 1);
}

#[tokio::test]
async fn resolve_mcp_auth_falls_back_to_the_static_token() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut static_server = base_server();
    static_server.fields.authentication_token = Some(secret_credential("static"));
    let resolved = resolve_mcp_auth(&static_server, None, &cache)
        .await
        .expect("the static token resolves");
    match resolved {
        Some(McpAuthValue::Text(value)) => assert_eq!(value.expose_secret(), "static"),
        Some(McpAuthValue::Headers(_)) | None => panic!("the static token is a text value"),
    }
    let empty = McpAuthOverride::Text(SecretString::from(String::new()));
    let resolved = resolve_mcp_auth(&static_server, Some(&empty), &cache)
        .await
        .expect("the empty override falls through");
    match resolved {
        Some(McpAuthValue::Text(value)) => assert_eq!(value.expose_secret(), "static"),
        Some(McpAuthValue::Headers(_)) | None => panic!("the static token is a text value"),
    }
    assert_eq!(
        resolve_mcp_auth(&base_server(), None, &cache)
            .await
            .expect("no auth resolves"),
        None
    );
}

#[test]
fn resolved_token_header_yields_to_the_caller_override() {
    let mut server = base_server();
    server.fields.upstream_token_header = Some("x-upstream-token".to_owned());
    let override_header = McpAuthOverride::Text(SecretString::from("caller".to_owned()));
    assert_eq!(resolved_token_header(&server, Some(&override_header)), None);
    assert_eq!(
        resolved_token_header(&server, None),
        Some("x-upstream-token")
    );
    let empty = McpAuthOverride::Text(SecretString::from(String::new()));
    assert_eq!(
        resolved_token_header(&server, Some(&empty)),
        Some("x-upstream-token")
    );
    assert_eq!(resolved_token_header(&base_server(), None), None);
}

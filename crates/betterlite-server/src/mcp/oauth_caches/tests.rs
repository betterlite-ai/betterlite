#![cfg(test)]

mod per_user;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTokenEndpointAuthMethod};
use betterlite_mcp::server_record::{MCPServer, OAuth2Flow, SecretCredential};
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::future::join_all;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::auth::{McpAuthOverride, McpAuthValue};
use super::error::OAuthCacheError;
use super::request::auth_method_name;
use super::{MCPOAuth2TokenCache, resolve_mcp_auth, resolved_token_header};
use crate::mcp::outbound_credentials::WallClock;
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
    let poster: Arc<dyn M2mTokenEndpointPost> = Arc::<ScriptedPost>::clone(post);
    let wall_clock: Arc<dyn WallClock> = Arc::<ManualClock>::clone(clock);
    MCPOAuth2TokenCache::new(poster, wall_clock).expect("the cache builds")
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

type ServerMutation = fn(&mut MCPServer);

fn with_changed_client(server: &mut MCPServer) {
    server.fields.client_id = Some("other".to_owned());
}

fn with_rotated_secret(server: &mut MCPServer) {
    server.fields.client_secret = Some(secret_credential("rotated"));
}

fn with_retargeted_url(server: &mut MCPServer) {
    server.fields.token_url = Some("https://other.example.com/token".to_owned());
}

fn with_narrowed_scopes(server: &mut MCPServer) {
    server.fields.scopes = Some(vec!["read".to_owned()]);
}

fn with_upstream_resource(server: &mut MCPServer) {
    server.fields.upstream_resource = Some("https://api.example.com".to_owned());
}

fn with_basic_auth_method(server: &mut MCPServer) {
    server.fields.token_endpoint_auth_method = Some(MCPTokenEndpointAuthMethod::ClientSecretBasic);
}

fn with_automatic_resource(server: &mut MCPServer) {
    server.fields.url = Some("https://mcp.example.com/sse".to_owned());
    server.fields.upstream_resource = Some("auto".to_owned());
}

#[rstest]
#[case::changed_client(with_changed_client)]
#[case::rotated_secret(with_rotated_secret)]
#[case::retargeted_url(with_retargeted_url)]
#[case::narrowed_scopes(with_narrowed_scopes)]
#[case::upstream_resource(with_upstream_resource)]
#[case::basic_auth_method(with_basic_auth_method)]
#[case::automatic_resource(with_automatic_resource)]
fn token_identity_changes_with_any_request_material(#[case] mutate: ServerMutation) {
    let base = MCPOAuth2TokenCache::token_identity(&client_server());
    let mut server = client_server();
    mutate(&mut server);
    assert_ne!(MCPOAuth2TokenCache::token_identity(&server), base);
}

#[test]
fn auth_method_names_match_litellm() {
    assert_eq!(
        auth_method_name(MCPTokenEndpointAuthMethod::ClientSecretBasic),
        "client_secret_basic"
    );
    assert_eq!(
        auth_method_name(MCPTokenEndpointAuthMethod::ClientSecretPost),
        "client_secret_post"
    );
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
    // LiteLLM expires an entry only when the clock passes its expiry, so the boundary second still serves it.
    clock.advance(1);
    let on_the_boundary = cache
        .async_get_token(&server)
        .await
        .expect("the boundary read succeeds");
    assert_eq!(on_the_boundary, Some("tok-0".to_owned()));
    assert_eq!(post.calls().len(), 1);
    clock.advance(1);
    let refetched = cache
        .async_get_token(&server)
        .await
        .expect("the second fetch succeeds");
    assert_eq!(refetched, Some("tok-1".to_owned()));
    assert_eq!(post.calls().len(), 2);
}

fn without_flow(server: &mut MCPServer) {
    server.fields.oauth2_flow = None;
}

fn with_authorization_code(server: &mut MCPServer) {
    server.fields.oauth2_flow = Some(OAuth2Flow::AuthorizationCode);
}

fn without_secret(server: &mut MCPServer) {
    server.fields.client_secret = None;
}

fn with_empty_secret(server: &mut MCPServer) {
    server.fields.client_secret = Some(secret_credential(""));
}

fn with_empty_client_id(server: &mut MCPServer) {
    server.fields.client_id = Some(String::new());
}

fn without_token_url(server: &mut MCPServer) {
    server.fields.token_url = None;
}

#[rstest]
#[case::missing_flow(without_flow)]
#[case::authorization_code(with_authorization_code)]
#[case::missing_secret(without_secret)]
#[case::empty_secret(with_empty_secret)]
#[case::empty_client_id(with_empty_client_id)]
#[case::missing_token_url(without_token_url)]
#[tokio::test]
async fn async_get_token_returns_none_without_client_credentials_config(
    #[case] mutate: ServerMutation,
) {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let mut server = client_server();
    mutate(&mut server);
    assert_eq!(
        cache.async_get_token(&server).await.expect("no fetch runs"),
        None
    );
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
    let results = join_all((0..8).map(|_index| cache.async_get_token(&server))).await;
    assert_eq!(post.calls().len(), 1);
    let tokens = results
        .into_iter()
        .map(|result| result.expect("every caller gets a token"))
        .collect::<Vec<_>>();
    assert_eq!(tokens, vec![Some("tok".to_owned()); 8]);
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

#[rstest]
#[case::absent(r#"{"access_token":"tok"}"#, 3540)]
#[case::null(r#"{"access_token":"tok","expires_in":null}"#, 3540)]
#[case::text_number(r#"{"access_token":"tok","expires_in":"120"}"#, 60)]
#[case::padded_text_number(r#"{"access_token":"tok","expires_in":" 36 "}"#, 10)]
#[case::text_word(r#"{"access_token":"tok","expires_in":"soon"}"#, 3540)]
#[case::float(r#"{"access_token":"tok","expires_in":1.5}"#, 10)]
#[case::flag(r#"{"access_token":"tok","expires_in":true}"#, 10)]
#[case::under_the_minimum(r#"{"access_token":"tok","expires_in":30}"#, 10)]
#[case::on_the_minimum(r#"{"access_token":"tok","expires_in":10}"#, 10)]
#[case::below_the_buffer(r#"{"access_token":"tok","expires_in":70}"#, 10)]
#[case::above_the_buffer(r#"{"access_token":"tok","expires_in":71}"#, 11)]
#[tokio::test]
async fn fetch_token_buffers_and_parses_expires_in(#[case] body: &'static str, #[case] ttl: i64) {
    let post = Arc::new(ScriptedPost::new(vec![success(body)]));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let fetched = cache
        .fetch_token(&client_server())
        .await
        .expect("the fetch succeeds");
    assert_eq!(fetched.1, ttl, "for {body}");
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
    let errors = join_all((0..3).map(|_index| cache.fetch_token(&server)))
        .await
        .into_iter()
        .map(|result| result.expect_err("the response has no usable access token"))
        .collect::<Vec<_>>();
    assert!(
        errors
            .iter()
            .all(|error| matches!(error, OAuthCacheError::MissingAccessToken { .. }))
    );
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
    cache
        .async_get_token(&first)
        .await
        .expect("the fetch succeeds");
    cache
        .async_get_token(&second)
        .await
        .expect("the fetch succeeds");
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
fn the_cache_evicts_the_earliest_expiry_at_the_max_size() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    (0..super::OAUTH2_TOKEN_CACHE_MAX_SIZE).for_each(|index| {
        let ttl = if index == 0 { 60 } else { 600 };
        cache
            .insert(&format!("srv-1:{index}"), "token", ttl)
            .expect("the entry fits");
    });
    assert_eq!(
        cache.cached("srv-1:0").expect("the cache reads"),
        Some("token".to_owned())
    );
    cache
        .insert("srv-1:new", "fresh", 600)
        .expect("the entry fits");
    assert_eq!(cache.cached("srv-1:0").expect("the cache reads"), None);
    assert_eq!(
        cache.cached("srv-1:new").expect("the cache reads"),
        Some("fresh".to_owned())
    );
    assert_eq!(
        cache.cached("srv-1:1").expect("the cache reads"),
        Some("token".to_owned())
    );
}

#[test]
fn the_cache_keeps_an_existing_identity_at_the_max_size() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    (0..super::OAUTH2_TOKEN_CACHE_MAX_SIZE).for_each(|index| {
        cache
            .insert(&format!("srv-1:{index}"), "token", 600)
            .expect("the entry fits");
    });
    let last = format!("srv-1:{}", super::OAUTH2_TOKEN_CACHE_MAX_SIZE - 1);
    cache.insert(&last, "fresh", 600).expect("the entry fits");
    // Equal expiries evict the smallest key first, and the overwritten identity keeps its place with the new token.
    assert_eq!(cache.cached("srv-1:0").expect("the cache reads"), None);
    assert_eq!(
        cache.cached(&last).expect("the cache reads"),
        Some("fresh".to_owned())
    );
    // An overwrite at capacity still evicts the earliest expiry first, so the cache ends one entry short.
    let entries = cache
        .entries
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .ttl_entries();
    assert_eq!(entries.len(), super::OAUTH2_TOKEN_CACHE_MAX_SIZE - 1);
}

#[test]
fn the_lock_map_is_capped_and_drops_the_oldest_identity() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    (0..=super::OAUTH2_TOKEN_CACHE_MAX_LOCKS).for_each(|index| {
        let _lock = cache.lock_for(&format!("srv-1:{index}"));
    });
    let locks = cache.locks.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(locks.len(), 1024);
    assert!(!locks.contains_key("srv-1:0"));
    assert!(locks.contains_key("srv-1:1"));
    assert!(locks.contains_key(&format!("srv-1:{}", super::OAUTH2_TOKEN_CACHE_MAX_LOCKS)));
}

#[test]
fn the_cache_matches_litellm_inmemory_defaults() {
    let post = Arc::new(ScriptedPost::new(Vec::new()));
    let clock = Arc::new(ManualClock::new());
    let cache = cache_with(&post, &clock);
    let entries = cache.entries.lock().unwrap_or_else(PoisonError::into_inner);
    let limits = entries.as_ref();
    assert_eq!(limits.capacity, 200);
    assert!((limits.default_ttl - 3600.0_f64).abs() < f64::EPSILON);
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
                    .map(|value| value.expose_secret().to_owned()),
                Some("caller-key".to_owned())
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
    let fell_through = resolve_mcp_auth(&static_server, Some(&empty), &cache)
        .await
        .expect("the empty override falls through");
    match fell_through {
        Some(McpAuthValue::Text(value)) => assert_eq!(value.expose_secret(), "static"),
        Some(McpAuthValue::Headers(_)) | None => panic!("the static token is a text value"),
    }
    assert!(
        resolve_mcp_auth(&base_server(), None, &cache)
            .await
            .expect("no auth resolves")
            .is_none()
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

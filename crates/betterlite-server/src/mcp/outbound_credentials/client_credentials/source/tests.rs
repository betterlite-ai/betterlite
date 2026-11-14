#![cfg(test)]

use std::collections::{HashMap, VecDeque};
use std::iter;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::errors::CredError;
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use chrono::{DateTime, TimeZone as _, Utc};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::{ClientCredentialsTokenSource, Tunables};
use crate::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome, TokenResponseBody,
};
use crate::mcp::outbound_credentials::{OAuthToken, TokenCacheBackend, TokenKey, WallClock};

const NOW: i64 = 1_000_000;

struct ManualClock(Mutex<DateTime<Utc>>);

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn clock() -> Arc<ManualClock> {
    Arc::new(ManualClock(Mutex::new(Utc.timestamp_opt(NOW, 0).unwrap())))
}

#[derive(Default)]
struct RecordingBackend {
    entries: Mutex<HashMap<TokenKey, OAuthToken>>,
    writes: Mutex<Vec<(TokenKey, Duration)>>,
    deletes: Mutex<Vec<TokenKey>>,
}

impl RecordingBackend {
    fn write_ttls(&self) -> Vec<Duration> {
        let writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        writes.iter().map(|(_, ttl)| *ttl).collect()
    }

    fn keys(&self) -> Vec<TokenKey> {
        let writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        writes.iter().map(|(key, _)| key.clone()).collect()
    }

    fn deletion_count(&self) -> usize {
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl TokenCacheBackend for RecordingBackend {
    async fn get(&self, key: &TokenKey) -> Option<OAuthToken> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set(&self, key: &TokenKey, token: &OAuthToken, ttl: Duration) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), token.clone());
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.clone(), ttl));
    }

    async fn delete(&self, key: &TokenKey) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.clone());
    }
}

type RecordedCall = (String, Vec<(String, String)>);

struct ScriptedPost {
    outcomes: Mutex<VecDeque<TokenEndpointOutcome>>,
    calls: Mutex<Vec<RecordedCall>>,
    delay: Duration,
}

impl ScriptedPost {
    fn new(outcomes: Vec<TokenEndpointOutcome>) -> Arc<Self> {
        Arc::new(Self {
            outcomes: Mutex::new(VecDeque::from(outcomes)),
            calls: Mutex::new(Vec::new()),
            delay: Duration::ZERO,
        })
    }

    fn granting(count: usize) -> Arc<Self> {
        Self::new(grant_from(count))
    }

    fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn requested_form(&self, index: usize) -> Vec<(String, String)> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls[index].1.clone()
    }
}

#[async_trait]
impl M2mTokenEndpointPost for ScriptedPost {
    async fn post(
        &self,
        url: &str,
        form: &[(String, SecretString)],
        _headers: &[(String, SecretString)],
    ) -> TokenEndpointOutcome {
        sleep(self.delay).await;
        let recorded: Vec<(String, String)> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_owned(), recorded));
        let mut outcomes = self.outcomes.lock().unwrap_or_else(PoisonError::into_inner);
        outcomes
            .pop_front()
            .expect("the script has an outcome left")
    }
}

fn body(raw: &str) -> TokenResponseBody {
    serde_json::from_str(raw).unwrap()
}

fn grant(raw: &str) -> TokenEndpointOutcome {
    TokenEndpointOutcome::Success { body: body(raw) }
}

fn grant_from(count: usize) -> Vec<TokenEndpointOutcome> {
    (0..count)
        .map(|index| grant(&format!("{{\"access_token\":\"tok-{index}\"}}")))
        .collect()
}

fn config() -> ClientCredentialsConfig {
    ClientCredentialsConfig {
        header: HeaderCarrier::default(),
        client_id: Some("client".to_owned()),
        client_secret: Some(SecretString::from("secret".to_owned())),
        token_url: Some("https://idp.example.com/token".to_owned()),
        scopes: vec!["read".to_owned()],
        audience: None,
        upstream_resource: None,
        token_endpoint_auth_method: None,
    }
}

fn source_with(
    post: Arc<ScriptedPost>,
    backend: Arc<RecordingBackend>,
) -> ClientCredentialsTokenSource {
    ClientCredentialsTokenSource::new(post, Some(backend), clock(), Tunables::default())
}

fn token(raw: &str) -> SecretString {
    SecretString::from(raw.to_owned())
}

#[tokio::test]
async fn get_serves_a_cached_token_without_another_grant() {
    let post = ScriptedPost::granting(1);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(Arc::clone(&post), backend);
    let first = source.get("server", &config()).await.unwrap();
    let second = source.get("server", &config()).await.unwrap();
    assert_eq!(first.access_token, "tok-0");
    assert_eq!(second.access_token, "tok-0");
    assert_eq!(post.call_count(), 1);
}

#[tokio::test]
async fn the_default_backend_caches_with_the_sources_clock() {
    let post = ScriptedPost::granting(1);
    let source = ClientCredentialsTokenSource::new(post, None, clock(), Tunables::default());
    source.get("server", &config()).await.unwrap();
    source.get("server", &config()).await.unwrap();
}

#[tokio::test]
async fn concurrent_misses_on_one_server_make_one_grant() {
    let delayed = Arc::new(ScriptedPost {
        outcomes: Mutex::new(VecDeque::from(grant_from(1))),
        calls: Mutex::new(Vec::new()),
        delay: Duration::from_millis(20),
    });
    let source = Arc::new(source_with(
        Arc::clone(&delayed),
        Arc::new(RecordingBackend::default()),
    ));
    let mut tasks = JoinSet::new();
    let handles: Vec<_> = iter::repeat_with(|| {
        let task_source = Arc::clone(&source);
        tasks.spawn(async move { task_source.get("server", &config()).await })
    })
    .take(10)
    .collect();
    assert_eq!(handles.len(), 10);
    let mut joined = Vec::new();
    while let Some(result) = tasks.join_next().await {
        joined.push(result);
    }
    assert_eq!(joined.len(), 10);
    assert!(
        joined
            .into_iter()
            .all(|result| result.unwrap().unwrap().access_token == "tok-0")
    );
    assert_eq!(delayed.call_count(), 1);
}

#[tokio::test]
async fn the_lock_table_drops_the_oldest_entry_at_max_locks() {
    let post = ScriptedPost::granting(3);
    let tunables = Tunables {
        default_ttl_seconds: 300,
        expiry_skew_seconds: 60,
        min_cache_seconds: 10,
        max_locks: 2,
    };
    let source = ClientCredentialsTokenSource::new(
        post,
        Some(Arc::new(RecordingBackend::default())),
        clock(),
        tunables,
    );
    source.get("s0", &config()).await.unwrap();
    source.get("s1", &config()).await.unwrap();
    source.get("s2", &config()).await.unwrap();
    let (count, present) = {
        let locks = source.locks.lock().unwrap_or_else(PoisonError::into_inner);
        (
            locks.len(),
            [
                locks.contains_key("s0"),
                locks.contains_key("s1"),
                locks.contains_key("s2"),
            ],
        )
    };
    assert_eq!(count, 2);
    assert_eq!(present, [false, true, true]);
}

#[tokio::test]
async fn rotating_the_secret_mints_a_new_token() {
    let post = ScriptedPost::new(vec![
        grant("{\"access_token\":\"tok-0\"}"),
        grant("{\"access_token\":\"tok-1\"}"),
    ]);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(Arc::clone(&post), Arc::clone(&backend));
    assert_eq!(
        source.get("server", &config()).await.unwrap().access_token,
        "tok-0"
    );
    let mut rotated = config();
    rotated.client_secret = Some(token("rotated"));
    assert_eq!(
        source.get("server", &rotated).await.unwrap().access_token,
        "tok-1"
    );
    assert_eq!(post.call_count(), 2);
    let keys = backend.keys();
    assert_ne!(keys[0].user_id, keys[1].user_id);
}

#[tokio::test]
async fn changing_the_scopes_mints_a_new_token() {
    let post = ScriptedPost::new(vec![
        grant("{\"access_token\":\"tok-0\"}"),
        grant("{\"access_token\":\"tok-1\"}"),
    ]);
    let source = source_with(Arc::clone(&post), Arc::new(RecordingBackend::default()));
    assert_eq!(
        source.get("server", &config()).await.unwrap().access_token,
        "tok-0"
    );
    let mut rescoped = config();
    rescoped.scopes = vec!["admin".to_owned()];
    assert_eq!(
        source.get("server", &rescoped).await.unwrap().access_token,
        "tok-1"
    );
    assert_eq!(post.call_count(), 2);
}

#[tokio::test]
async fn two_servers_with_the_same_identity_each_get_a_grant() {
    let post = ScriptedPost::granting(2);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(Arc::clone(&post), Arc::clone(&backend));
    source.get("first", &config()).await.unwrap();
    source.get("second", &config()).await.unwrap();
    assert_eq!(post.call_count(), 2);
    assert_eq!(backend.keys()[0].user_id, backend.keys()[1].user_id);
    assert_ne!(backend.keys()[0].server_id, backend.keys()[1].server_id);
}

#[tokio::test]
async fn an_unreachable_endpoint_is_upstream_unavailable() {
    let post = ScriptedPost::new(vec![TokenEndpointOutcome::Unreachable {
        detail: "Connect".to_owned(),
    }]);
    let source = source_with(post, Arc::new(RecordingBackend::default()));
    let failure = source.get("server", &config()).await.unwrap_err();
    assert_eq!(
        failure,
        CredError::UpstreamUnavailable("OAuth2 token endpoint unreachable: Connect".to_owned())
    );
}

#[tokio::test]
async fn a_server_error_is_upstream_unavailable() {
    let post = ScriptedPost::new(vec![TokenEndpointOutcome::Denied {
        status_code: 503,
        detail: "token endpoint returned HTTP 503".to_owned(),
    }]);
    let source = source_with(post, Arc::new(RecordingBackend::default()));
    let failure = source.get("server", &config()).await.unwrap_err();
    assert_eq!(
        failure,
        CredError::UpstreamUnavailable(
            "OAuth2 token endpoint failed: token endpoint returned HTTP 503".to_owned()
        )
    );
}

#[tokio::test]
async fn a_client_error_is_misconfigured() {
    let post = ScriptedPost::new(vec![TokenEndpointOutcome::Denied {
        status_code: 400,
        detail: "token endpoint returned HTTP 400".to_owned(),
    }]);
    let source = source_with(post, Arc::new(RecordingBackend::default()));
    let failure = source.get("server", &config()).await.unwrap_err();
    assert_eq!(
        failure,
        CredError::Misconfigured(
            "OAuth2 client_credentials grant rejected: token endpoint returned HTTP 400".to_owned()
        )
    );
}

#[rstest]
#[case::absent("{}")]
#[case::empty("{\"access_token\":\"\"}")]
#[case::number("{\"access_token\":7}")]
#[tokio::test]
async fn a_body_without_a_usable_token_is_misconfigured(#[case] raw: &str) {
    let post = ScriptedPost::new(vec![grant(raw)]);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(post, Arc::clone(&backend));
    let failure = source.get("server", &config()).await.unwrap_err();
    assert_eq!(
        failure,
        CredError::Misconfigured("OAuth2 token response is missing 'access_token'".to_owned())
    );
    assert_eq!(backend.len(), 0);
}

#[rstest]
#[case::no_expiry("{\"access_token\":\"tok\"}", Duration::from_secs(300))]
#[case::skewed(
    "{\"access_token\":\"tok\",\"expires_in\":3600}",
    Duration::from_mins(59)
)]
#[case::below_the_floor("{\"access_token\":\"tok\",\"expires_in\":5}", Duration::from_secs(5))]
#[case::at_the_floor(
    "{\"access_token\":\"tok\",\"expires_in\":10}",
    Duration::from_secs(10)
)]
#[case::at_the_skew(
    "{\"access_token\":\"tok\",\"expires_in\":70}",
    Duration::from_secs(10)
)]
#[case::above_the_skew(
    "{\"access_token\":\"tok\",\"expires_in\":71}",
    Duration::from_secs(11)
)]
#[case::float_falls_back(
    "{\"access_token\":\"tok\",\"expires_in\":3600.5}",
    Duration::from_secs(300)
)]
#[tokio::test]
async fn the_cache_ttl_follows_the_skew_and_the_floor(
    #[case] raw: &str,
    #[case] expected: Duration,
) {
    let post = ScriptedPost::new(vec![grant(raw)]);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(post, Arc::clone(&backend));
    source.get("server", &config()).await.unwrap();
    assert_eq!(backend.write_ttls(), vec![expected]);
}

#[rstest]
#[case::zero("{\"access_token\":\"tok\",\"expires_in\":0}")]
#[case::negative("{\"access_token\":\"tok\",\"expires_in\":-5}")]
#[tokio::test]
async fn a_non_positive_ttl_stores_nothing_but_still_returns_the_token(#[case] raw: &str) {
    let post = ScriptedPost::new(vec![grant(raw)]);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(post, Arc::clone(&backend));
    let token = source.get("server", &config()).await.unwrap();
    assert_eq!(token.access_token, "tok");
    assert!(backend.write_ttls().is_empty());
    assert_eq!(backend.len(), 0);
}

#[tokio::test]
async fn a_parsed_expires_in_becomes_the_expiry_and_the_response_scope_the_scopes() {
    let raw = "{\"access_token\":\"tok\",\"expires_in\":\"3600\",\"scope\":\" read  write \"}";
    let post = ScriptedPost::new(vec![grant(raw)]);
    let source = source_with(post, Arc::new(RecordingBackend::default()));
    let token = source.get("server", &config()).await.unwrap();
    assert_eq!(
        token.expires_at,
        Some(Utc.timestamp_opt(NOW + 3600, 0).unwrap())
    );
    assert_eq!(token.scopes, vec!["read".to_owned(), "write".to_owned()]);
    assert_eq!(token.refresh_token, None);
    assert_eq!(token.identity_binding_proof, None);
}

#[rstest]
#[case::bool("{\"access_token\":\"tok\",\"expires_in\":true}")]
#[case::float("{\"access_token\":\"tok\",\"expires_in\":3600.5}")]
#[tokio::test]
async fn an_unparsable_expires_in_leaves_the_expiry_absent(#[case] raw: &str) {
    let post = ScriptedPost::new(vec![grant(raw)]);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(post, Arc::clone(&backend));
    let token = source.get("server", &config()).await.unwrap();
    assert_eq!(token.expires_at, None);
    assert_eq!(backend.write_ttls(), vec![Duration::from_secs(300)]);
}

#[test]
fn an_expires_in_past_the_clock_range_saturates_the_expiry() {
    let source = source_with(
        ScriptedPost::granting(0),
        Arc::new(RecordingBackend::default()),
    );
    assert_eq!(source.expiry(i64::MAX), DateTime::<Utc>::MAX_UTC);
}

#[tokio::test]
async fn refetch_returns_a_token_another_caller_already_replaced() {
    let post = ScriptedPost::granting(1);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(Arc::clone(&post), backend);
    source.get("server", &config()).await.unwrap();
    let fresh = source
        .refetch("server", &config(), &token("tok-stale"))
        .await;
    assert_eq!(
        fresh.map(|value| value.expose_secret().to_owned()),
        Some("tok-0".to_owned())
    );
    assert_eq!(post.call_count(), 1);
}

#[tokio::test]
async fn refetch_replaces_the_failed_token_with_a_new_grant() {
    let post = ScriptedPost::granting(2);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(Arc::clone(&post), Arc::clone(&backend));
    source.get("server", &config()).await.unwrap();
    let fresh = source.refetch("server", &config(), &token("tok-0")).await;
    assert_eq!(
        fresh.map(|value| value.expose_secret().to_owned()),
        Some("tok-1".to_owned())
    );
    assert_eq!(post.call_count(), 2);
    assert_eq!(backend.deletion_count(), 1);
    assert_eq!(backend.len(), 1);
}

#[tokio::test]
async fn a_failed_refetch_leaves_the_cache_empty_and_returns_none() {
    let post = ScriptedPost::new(vec![
        grant("{\"access_token\":\"tok-0\"}"),
        TokenEndpointOutcome::Denied {
            status_code: 401,
            detail: "token endpoint returned HTTP 401".to_owned(),
        },
    ]);
    let backend = Arc::new(RecordingBackend::default());
    let source = source_with(Arc::clone(&post), Arc::clone(&backend));
    source.get("server", &config()).await.unwrap();
    let fresh = source.refetch("server", &config(), &token("tok-0")).await;
    assert!(fresh.is_none());
    assert_eq!(post.call_count(), 2);
    assert_eq!(backend.len(), 0);
}

#[tokio::test]
async fn refetch_without_a_cached_token_grants_one() {
    let post = ScriptedPost::granting(1);
    let source = source_with(Arc::clone(&post), Arc::new(RecordingBackend::default()));
    let fresh = source.refetch("server", &config(), &token("tok-0")).await;
    assert_eq!(
        fresh.map(|value| value.expose_secret().to_owned()),
        Some("tok-0".to_owned())
    );
    assert_eq!(post.call_count(), 1);
}

#[tokio::test]
async fn refetch_with_an_incomplete_config_returns_none_without_a_grant() {
    let post = ScriptedPost::granting(0);
    let source = source_with(Arc::clone(&post), Arc::new(RecordingBackend::default()));
    let mut incomplete = config();
    incomplete.client_id = None;
    assert!(
        source
            .refetch("server", &incomplete, &token("tok-0"))
            .await
            .is_none()
    );
    assert_eq!(post.call_count(), 0);
}

#[tokio::test]
async fn get_with_an_incomplete_config_returns_the_missing_field_error() {
    let post = ScriptedPost::granting(0);
    let source = source_with(Arc::clone(&post), Arc::new(RecordingBackend::default()));
    let mut incomplete = config();
    incomplete.token_url = Some(String::new());
    let failure = source.get("server", &incomplete).await.unwrap_err();
    assert_eq!(
        failure,
        CredError::Misconfigured("client_credentials config is missing: token_url".to_owned())
    );
    assert_eq!(post.call_count(), 0);
}

#[tokio::test]
async fn the_grant_carries_the_prepared_form_and_headers() {
    let post = ScriptedPost::granting(1);
    let source = source_with(Arc::clone(&post), Arc::new(RecordingBackend::default()));
    source.get("server", &config()).await.unwrap();
    assert_eq!(
        post.requested_form(0),
        vec![
            ("grant_type".to_owned(), "client_credentials".to_owned()),
            ("client_id".to_owned(), "client".to_owned()),
            ("client_secret".to_owned(), "secret".to_owned()),
            ("scope".to_owned(), "read".to_owned()),
        ]
    );
}

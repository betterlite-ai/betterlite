#![cfg(test)]

use std::collections::BTreeMap;
use std::str::from_utf8;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::routing::post;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_user_credentials::store::get_user_oauth_credential;
use betterlite_db::mcp_user_credentials::store_user_oauth_credential;
use betterlite_db::mcp_user_credentials::types::StoreOAuthCredential;
use betterlite_db::oauth_token_resolution::ports::TokenResponse;
use betterlite_db::oauth_token_resolution::{EnforceRequest, OauthBindingPort as _};
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_mcp::server_record::MCPServer;
use chrono::TimeDelta;
use http::Method;
use http::header::ACCEPT;
use rstest::rstest;
use secrecy::SecretString;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::adapters::{DbCredentialPersist, DbCredentialReader, PooledTokenPost, refresh_request};
use super::chain::{SharedBinding, StoreBuilder as _};
use crate::mcp::outbound_credentials::authz_code_refresher::ports::{
    CredentialPersist as _, PersistRequest, TokenEndpointPost as _,
};
use crate::mcp::outbound_credentials::v2_store::{
    CredentialPayload, CredentialReader as _, CredentialValue,
};
use crate::mcp::outbound_credentials::{
    InvalidatableOAuthTokenStore as _, OAuthTokenStore as _, PER_USER_TOKEN_KEY_PREFIX,
    iso_to_epoch,
};
use support::{
    ChainWiring, DEADLINE, FailingRows, FakeRows, FixedWallClock, RecordingBinding,
    answer_with_array, answer_with_object, answer_with_server_error, base_time, chain_fixture,
    expired_credential, exposed, grant, lazy_fixture, lazy_token, oauth_server, pooled_client,
    reader_fixture, sealed_byok, serving_token_endpoint, test_cipher,
};

mod support;

#[test]
fn adapter_pooled_post_defaults_the_accept_header_and_form_encodes_the_grant() {
    let form = BTreeMap::from([
        ("grant_type".to_owned(), "refresh_token".to_owned()),
        ("refresh_token".to_owned(), "a b".to_owned()),
    ]);
    let request =
        refresh_request("https://idp.test/token", &form, &BTreeMap::new(), DEADLINE).unwrap();
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.headers.get(ACCEPT).unwrap().to_str().unwrap(),
        "application/json"
    );
    let RequestBody::Bytes { content_type, data } = &request.body else {
        panic!("a refresh carries a form body");
    };
    assert_eq!(
        content_type.to_str().unwrap(),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(
        from_utf8(data).unwrap(),
        "grant_type=refresh_token&refresh_token=a+b"
    );
}

#[test]
fn adapter_refresh_request_lets_a_given_accept_win_and_marks_the_headers_sensitive() {
    let form = BTreeMap::from([("grant_type".to_owned(), "refresh_token".to_owned())]);
    let given = BTreeMap::from([
        ("Accept".to_owned(), "text/plain".to_owned()),
        ("x-api-key".to_owned(), "code".to_owned()),
    ]);
    let request = refresh_request("https://idp.test/token", &form, &given, DEADLINE).unwrap();
    assert_eq!(
        request.headers.get(ACCEPT).unwrap().to_str().unwrap(),
        "text/plain"
    );
    assert!(request.headers.get("x-api-key").unwrap().is_sensitive());
}

#[test]
fn adapter_refresh_request_rejects_an_invalid_header_name_or_value() {
    let form = BTreeMap::from([("grant_type".to_owned(), "refresh_token".to_owned())]);
    let bad_name = BTreeMap::from([("bad header".to_owned(), "x".to_owned())]);
    let Err(_name_error) = refresh_request("https://idp.test/token", &form, &bad_name, DEADLINE)
    else {
        panic!("a header name with a space is rejected");
    };
    let bad_value = BTreeMap::from([("x-api-key".to_owned(), "a\nb".to_owned())]);
    let Err(_value_error) = refresh_request("https://idp.test/token", &form, &bad_value, DEADLINE)
    else {
        panic!("a header value with a newline is rejected");
    };
}

#[tokio::test]
async fn adapter_pooled_post_returns_the_json_object_body() {
    let (url, _tasks) =
        serving_token_endpoint(Router::new().route("/token", post(answer_with_object))).await;
    let token_post = PooledTokenPost {
        client: pooled_client(),
        deadline: DEADLINE,
    };
    let form = BTreeMap::from([("grant_type".to_owned(), "refresh_token".to_owned())]);
    let answer = token_post
        .post_form(&url, &form, &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        answer.get("access_token"),
        Some(&JsonValue::String("fresh".to_owned()))
    );
}

#[tokio::test]
async fn adapter_pooled_post_turns_a_non_2xx_answer_into_none() {
    let (url, _tasks) =
        serving_token_endpoint(Router::new().route("/token", post(answer_with_server_error))).await;
    let token_post = PooledTokenPost {
        client: pooled_client(),
        deadline: DEADLINE,
    };
    let form = BTreeMap::from([("grant_type".to_owned(), "refresh_token".to_owned())]);
    assert!(
        token_post
            .post_form(&url, &form, &BTreeMap::new())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn adapter_pooled_post_turns_a_non_object_body_into_none() {
    let (url, _tasks) =
        serving_token_endpoint(Router::new().route("/token", post(answer_with_array))).await;
    let token_post = PooledTokenPost {
        client: pooled_client(),
        deadline: DEADLINE,
    };
    let form = BTreeMap::from([("grant_type".to_owned(), "refresh_token".to_owned())]);
    assert!(
        token_post
            .post_form(&url, &form, &BTreeMap::new())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn adapter_pooled_post_turns_a_transport_error_into_none() {
    let token_post = PooledTokenPost {
        client: pooled_client(),
        deadline: DEADLINE,
    };
    let form = BTreeMap::from([("grant_type".to_owned(), "refresh_token".to_owned())]);
    assert!(
        token_post
            .post_form("http://127.0.0.1:1/token", &form, &BTreeMap::new())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn adapter_pooled_post_turns_an_unusable_request_into_none() {
    let token_post = PooledTokenPost {
        client: pooled_client(),
        deadline: DEADLINE,
    };
    let form = BTreeMap::from([("grant_type".to_owned(), "refresh_token".to_owned())]);
    assert!(
        token_post
            .post_form("not a url", &form, &BTreeMap::new())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn adapter_reader_reports_a_disconnected_database() {
    let credential_reader = DbCredentialReader::<FakeRows> {
        database: None,
        cipher: test_cipher(),
    };
    let error = credential_reader
        .read_credential("user", "srv")
        .await
        .unwrap_err();
    assert_eq!(error.detail, "Database not connected");
}

fn stored_payload(expires_at: CredentialValue) -> CredentialPayload {
    CredentialPayload {
        access_token: CredentialValue::Text("stored".to_owned()),
        refresh_token: CredentialValue::Null,
        expires_at,
        scopes: CredentialValue::Array(vec![CredentialValue::Text("read".to_owned())]),
        identity_binding_proof: CredentialValue::Text("proof".to_owned()),
    }
}

#[tokio::test]
async fn adapter_reader_maps_a_stored_row_into_the_chain_payload() {
    let cipher = test_cipher();
    let table = FakeRows::default();
    let access = SecretString::from("stored".to_owned());
    let scopes = ["read".to_owned()];
    let stored = StoreOAuthCredential {
        user_id: "user",
        server_id: "srv",
        access_token: &access,
        refresh_token: None,
        expires_in: Some(60),
        scopes: Some(&scopes),
        skip_byok_guard: true,
        identity_binding_proof: Some("proof"),
    };
    let base = base_time();
    store_user_oauth_credential(&table, &cipher, &stored, base)
        .await
        .unwrap();
    let credential_reader = DbCredentialReader {
        database: Some(table),
        cipher: Arc::clone(&cipher),
    };
    let credential = credential_reader
        .read_credential("user", "srv")
        .await
        .unwrap()
        .unwrap();
    let CredentialValue::Text(expires_at) = &credential.expires_at else {
        panic!("a stored grant carries an expires_at");
    };
    assert_eq!(
        iso_to_epoch(expires_at),
        base.checked_add_signed(TimeDelta::seconds(60))
    );
    assert_eq!(credential, stored_payload(credential.expires_at.clone()));
}

#[tokio::test]
async fn adapter_reader_reports_a_table_error_with_its_message() {
    let credential_reader = DbCredentialReader {
        database: Some(FailingRows),
        cipher: test_cipher(),
    };
    let error = credential_reader
        .read_credential("user", "srv")
        .await
        .unwrap_err();
    assert_eq!(error.detail, "the stored MCP server row has no server_id");
}

#[tokio::test]
async fn adapter_persist_skips_when_no_database_is_connected() {
    let credential_persist = DbCredentialPersist::<FakeRows> {
        database: None,
        cipher: test_cipher(),
        clock: Arc::new(FixedWallClock(base_time())),
    };
    let request = PersistRequest {
        user_id: "user",
        server_id: "srv",
        access_token: "fresh",
        refresh_token: None,
        expires_in: None,
        scopes: None,
        identity_binding_proof: None,
    };
    credential_persist.persist(&request).await.unwrap();
}

#[tokio::test]
async fn adapter_persist_writes_beside_a_readable_byok_row() {
    let cipher = test_cipher();
    let table = FakeRows::sealed(sealed_byok(&cipher));
    let base = base_time();
    let credential_persist = DbCredentialPersist {
        database: Some(table.clone()),
        cipher: Arc::clone(&cipher),
        clock: Arc::new(FixedWallClock(base)),
    };
    let scopes = ["read".to_owned(), "offline_access".to_owned()];
    let request = PersistRequest {
        user_id: "user",
        server_id: "srv",
        access_token: "fresh",
        refresh_token: Some("rotated"),
        expires_in: Some(3600),
        scopes: Some(&scopes),
        identity_binding_proof: Some("proof"),
    };
    credential_persist.persist(&request).await.unwrap();
    let payload = get_user_oauth_credential(&table, &cipher, "user", "srv")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exposed(payload.access_token.as_ref()), Some("fresh"));
    assert_eq!(exposed(payload.refresh_token.as_ref()), Some("rotated"));
    assert_eq!(payload.scopes, Some(scopes.to_vec()));
    assert_eq!(payload.identity_binding_proof.as_deref(), Some("proof"));
    assert_eq!(
        payload.expires_at.as_deref().and_then(iso_to_epoch),
        base.checked_add_signed(TimeDelta::seconds(3600))
    );
}

#[tokio::test]
async fn store_bound_read_hides_a_mismatched_credential_for_an_enforce_server() {
    let fixture = reader_fixture(
        Some(oauth_server(Some("enforce"))),
        Some("other-user"),
        false,
    );
    let credential = fixture.reader.read_credential("user", "srv").await.unwrap();
    assert!(credential.is_none());
    assert_eq!(fixture.binding.checks.load(Ordering::Relaxed), 1);
    assert_eq!(
        fixture.binding.proofs.lock().unwrap().clone(),
        vec![Some("other-user".to_owned())]
    );
}

#[tokio::test]
async fn store_bound_read_returns_a_matching_credential_for_an_enforce_server() {
    let fixture = reader_fixture(
        Some(oauth_server(Some("enforce"))),
        Some("user-proof"),
        true,
    );
    let credential = fixture
        .reader
        .read_credential("user", "srv")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        credential.identity_binding_proof,
        CredentialValue::Text("user-proof".to_owned())
    );
    assert_eq!(fixture.binding.checks.load(Ordering::Relaxed), 1);
    assert_eq!(
        fixture.binding.proofs.lock().unwrap().clone(),
        vec![Some("user-proof".to_owned())]
    );
}

#[rstest]
#[case::audit(Some(oauth_server(Some("audit"))))]
#[case::unbound(Some(oauth_server(None)))]
#[case::unknown(None)]
#[tokio::test]
async fn store_bound_read_passes_audit_unbound_and_unknown_servers_unchecked(
    #[case] server: Option<Arc<MCPServer>>,
) {
    let fixture = reader_fixture(server, Some("other-user"), false);
    let credential = fixture.reader.read_credential("user", "srv").await.unwrap();
    assert!(credential.is_some());
    assert_eq!(fixture.binding.checks.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn store_chain_without_redis_refreshes_and_caches_in_process() {
    let fixture = chain_fixture(
        Some(oauth_server(None)),
        Some(expired_credential()),
        false,
        Some(grant(Some(3600))),
    );
    let built = fixture.builder.build();
    assert!(!built.uses_redis);
    let first = built.store.fetch("user", "srv").await.unwrap().unwrap();
    assert_eq!(first.access_token, "fresh");
    assert_eq!(first.refresh_token.as_deref(), Some("rotated"));
    assert_eq!(fixture.post.calls.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.persist.calls.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.runs.load(Ordering::Relaxed), 0);
    let second = built.store.fetch("user", "srv").await.unwrap().unwrap();
    assert_eq!(second.access_token, "fresh");
    assert_eq!(
        fixture.post.calls.load(Ordering::Relaxed),
        1,
        "the in-process cache serves the second fetch without a refresh"
    );
}

#[tokio::test]
async fn store_chain_with_redis_uses_the_runtime_coordinator_and_default_ttl() {
    let fixture = chain_fixture(
        Some(oauth_server(None)),
        Some(expired_credential()),
        true,
        Some(grant(None)),
    );
    let built = fixture.builder.build();
    assert!(built.uses_redis);
    let token = built.store.fetch("user", "srv").await.unwrap().unwrap();
    assert_eq!(token.access_token, "fresh");
    assert_eq!(
        fixture.runs.load(Ordering::Relaxed),
        1,
        "the runtime refresh coordinator runs the refresh"
    );
    let key = format!("{PER_USER_TOKEN_KEY_PREFIX}user:srv");
    assert_eq!(
        fixture.cache.ttl_of(&key),
        Some(Some(Duration::from_secs(300))),
        "a token without an expiry uses the 300 second default TTL"
    );
    let cached = built.store.fetch("user", "srv").await.unwrap().unwrap();
    assert_eq!(cached.access_token, "fresh");
    assert_eq!(fixture.post.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn store_shared_binding_delegates_enforce_and_the_proof_check() {
    let server = oauth_server(None);
    let response = TokenResponse::new();
    let request = EnforceRequest {
        server: server.as_ref(),
        token_response: &response,
        user_id: Some("user"),
        grant_type: "refresh_token",
        ownership: None,
    };
    let shared = SharedBinding(Arc::new(RecordingBinding {
        matches: true,
        checks: AtomicUsize::new(0),
        proof: Some("bound".to_owned()),
        proofs: Mutex::new(Vec::new()),
    }));
    assert_eq!(
        shared.enforce(&request).await.unwrap(),
        Some("bound".to_owned())
    );
    assert!(
        shared
            .proof_matches(server.as_ref(), "user", Some("stored"))
            .await
    );
    let refusing = SharedBinding(Arc::new(RecordingBinding {
        matches: false,
        checks: AtomicUsize::new(0),
        proof: None,
        proofs: Mutex::new(Vec::new()),
    }));
    assert!(!refusing.proof_matches(server.as_ref(), "user", None).await);
}

#[tokio::test]
async fn store_lazy_builds_a_chain_on_the_first_fetch() {
    let fixture = lazy_fixture(
        None,
        Some(lazy_token("fresh", None)),
        true,
        ChainWiring::Local,
    );
    let fetched = fixture.store.fetch("user", "srv").await.unwrap().unwrap();
    assert_eq!(fetched.access_token, "fresh");
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.chain.fetches.load(Ordering::Relaxed), 1);
    drop(fixture.store.fetch("user", "srv").await.unwrap());
    assert_eq!(
        fixture.builds.load(Ordering::Relaxed),
        1,
        "the built chain is reused"
    );
    assert_eq!(fixture.chain.fetches.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn store_lazy_keeps_a_redis_chain() {
    let fixture = lazy_fixture(
        None,
        Some(lazy_token("fresh", None)),
        true,
        ChainWiring::Redis,
    );
    drop(fixture.store.fetch("user", "srv").await.unwrap());
    drop(fixture.store.fetch("user", "srv").await.unwrap());
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.chain.fetches.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn store_lazy_replaces_a_local_chain_once_redis_is_available() {
    let fixture = lazy_fixture(
        None,
        Some(lazy_token("fresh", None)),
        true,
        ChainWiring::Local,
    );
    drop(fixture.store.fetch("user", "srv").await.unwrap());
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 1);
    fixture.redis.store(true, Ordering::Relaxed);
    fixture.builder_redis.store(true, Ordering::Relaxed);
    drop(fixture.store.fetch("user", "srv").await.unwrap());
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 2);
    drop(fixture.store.fetch("user", "srv").await.unwrap());
    assert_eq!(
        fixture.builds.load(Ordering::Relaxed),
        2,
        "the redis chain is reused once built"
    );
}

#[tokio::test]
async fn store_lazy_waits_for_an_in_flight_local_fetch_before_replacing_the_chain() {
    let fixture = lazy_fixture(
        None,
        Some(lazy_token("fresh", None)),
        true,
        ChainWiring::Local,
    );
    fixture.chain.hold_first.store(true, Ordering::SeqCst);
    let started = Arc::clone(&fixture.chain.started);
    let release = Arc::clone(&fixture.chain.release);
    let shared = Arc::new(fixture.store);
    let first = Arc::clone(&shared);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { first.fetch("user", "srv").await });
    started.notified().await;
    fixture.redis.store(true, Ordering::Relaxed);
    fixture.builder_redis.store(true, Ordering::Relaxed);
    let second = shared.fetch("user", "srv");
    tokio::pin!(second);
    let completed_early = timeout(Duration::from_millis(20), &mut second)
        .await
        .is_ok();
    assert!(
        !completed_early,
        "a fetch arriving during the transition waits for the in-flight local fetch"
    );
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 1);
    release.notify_one();
    drop(tasks.join_next().await.unwrap().unwrap().unwrap());
    let second_token = second.await.unwrap().unwrap();
    assert_eq!(second_token.access_token, "fresh");
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 2);
    assert_eq!(fixture.chain.fetches.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn store_lazy_releases_a_cancelled_local_fetch() {
    let fixture = lazy_fixture(
        None,
        Some(lazy_token("fresh", None)),
        true,
        ChainWiring::Local,
    );
    fixture.chain.hold_first.store(true, Ordering::SeqCst);
    let started = Arc::clone(&fixture.chain.started);
    let shared = Arc::new(fixture.store);
    let first = Arc::clone(&shared);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { first.fetch("user", "srv").await });
    started.notified().await;
    tasks.shutdown().await;
    fixture.chain.hold_first.store(false, Ordering::SeqCst);
    fixture.redis.store(true, Ordering::Relaxed);
    fixture.builder_redis.store(true, Ordering::Relaxed);
    let fetched = timeout(DEADLINE, shared.fetch("user", "srv"))
        .await
        .expect("a cancelled local fetch releases its count")
        .unwrap()
        .unwrap();
    assert_eq!(fetched.access_token, "fresh");
    assert_eq!(
        fixture.builds.load(Ordering::Relaxed),
        2,
        "the cancelled local chain is replaced by the redis one"
    );
}

#[tokio::test]
async fn store_lazy_invalidate_builds_a_chain_when_none_exists() {
    let fixture = lazy_fixture(
        None,
        Some(lazy_token("fresh", None)),
        true,
        ChainWiring::Local,
    );
    fixture.store.invalidate("user", "srv").await;
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.chain.invalidations.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn store_lazy_invalidate_releases_its_local_fetch() {
    let fixture = lazy_fixture(
        None,
        Some(lazy_token("fresh", None)),
        true,
        ChainWiring::Local,
    );
    fixture.store.invalidate("user", "srv").await;
    fixture.redis.store(true, Ordering::Relaxed);
    fixture.builder_redis.store(true, Ordering::Relaxed);
    let fetched = timeout(DEADLINE, fixture.store.fetch("user", "srv"))
        .await
        .expect("an invalidate releases its local fetch")
        .unwrap()
        .unwrap();
    assert_eq!(fetched.access_token, "fresh");
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn store_lazy_exposes_the_redis_chain_once_built() {
    let fixture = lazy_fixture(
        None,
        Some(lazy_token("fresh", None)),
        true,
        ChainWiring::Redis,
    );
    assert!(fixture.store.redis_store().await.is_none());
    drop(fixture.store.fetch("user", "srv").await.unwrap());
    assert!(fixture.store.redis_store().await.is_some());
}

#[tokio::test]
async fn store_lazy_fetch_returns_none_and_invalidates_on_an_enforce_mismatch() {
    let fixture = lazy_fixture(
        Some(oauth_server(Some("enforce"))),
        Some(lazy_token("fresh", Some("other-user"))),
        false,
        ChainWiring::Local,
    );
    let fetched = fixture.store.fetch("user", "srv").await.unwrap();
    assert!(fetched.is_none());
    assert_eq!(fixture.chain.invalidations.lock().unwrap().len(), 1);
    assert_eq!(fixture.builds.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn store_lazy_fetch_returns_the_token_when_the_proof_matches() {
    let fixture = lazy_fixture(
        Some(oauth_server(Some("enforce"))),
        Some(lazy_token("fresh", Some("user-proof"))),
        true,
        ChainWiring::Local,
    );
    let fetched = fixture.store.fetch("user", "srv").await.unwrap().unwrap();
    assert_eq!(fetched.access_token, "fresh");
    assert!(fixture.chain.invalidations.lock().unwrap().is_empty());
}

#[rstest]
#[case::unknown(None)]
#[case::unbound(Some(oauth_server(None)))]
#[case::audit(Some(oauth_server(Some("audit"))))]
#[tokio::test]
async fn store_lazy_fetch_returns_the_token_for_unknown_unbound_and_audit_servers(
    #[case] server: Option<Arc<MCPServer>>,
) {
    let fixture = lazy_fixture(
        server,
        Some(lazy_token("fresh", Some("other-user"))),
        false,
        ChainWiring::Local,
    );
    let fetched = fixture.store.fetch("user", "srv").await.unwrap().unwrap();
    assert_eq!(fetched.access_token, "fresh");
    assert!(fixture.chain.invalidations.lock().unwrap().is_empty());
}

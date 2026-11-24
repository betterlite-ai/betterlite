#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_db::sso_assertion_rows::MemoryAssertionRows;
use betterlite_mcp::credential_contracts::byok_cache::SystemClock;
use betterlite_mcp::sso_assertions::cache::{CacheState, SsoAssertionCache};
use betterlite_mcp::sso_assertions::read::SsoAssertionStore as _;
use betterlite_mcp::sso_assertions::store::DbSsoAssertionStore;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use tokio::task::JoinSet;

use super::super::refresher::AssertionAccess as _;
use super::super::store::{RenewalCoordinator, SINGLE_FLIGHT_KEY};
use super::super::tests::support::{
    AccessFailure, NOW_MS, NOW_SECONDS, RecordingCoordinator, StoreFixture, access_with, assertion,
    claims, factory, failing_inner, id_token, inner, outage, rejected, renewed_body, transport,
    yielding_transport,
};

const RENEWED_EXP: i64 = NOW_SECONDS + 3600;

#[rstest]
#[case::far_from_expiry(NOW_SECONDS + 3600)]
#[case::just_past_the_skew(NOW_SECONDS + 61)]
#[tokio::test]
async fn a_fresh_assertion_is_returned_without_a_renewal(#[case] exp: i64) {
    let fresh = assertion(exp, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", fresh.clone())]),
        transport(Vec::new()),
        access_with(vec![("user-1", fresh.clone())], AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let found = fixture
        .store
        .fetch("user-1")
        .await
        .expect("the read succeeds")
        .expect("the row exists");
    assert_eq!(
        found.id_token.expose_secret(),
        fresh.id_token.expose_secret()
    );
    assert!(fixture.transport.sent().is_empty());
    assert_eq!(fixture.inner.fetches.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.factory.asked.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn an_absent_assertion_is_returned_without_a_renewal() {
    let fixture = StoreFixture::new(
        inner(Vec::new()),
        transport(Vec::new()),
        access_with(Vec::new(), AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let found = fixture
        .store
        .fetch("user-1")
        .await
        .expect("the read succeeds");
    assert!(found.is_none());
    assert!(fixture.transport.sent().is_empty());
    assert_eq!(fixture.factory.asked.load(Ordering::Relaxed), 0);
}

#[rstest]
#[case::inside_the_skew(NOW_SECONDS + 59)]
#[case::exactly_at_the_skew(NOW_SECONDS + 60)]
#[tokio::test]
async fn an_assertion_inside_the_skew_is_renewed_once_and_read_uncached(#[case] exp: i64) {
    let expired = assertion(exp, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", expired.clone())]),
        transport(vec![Ok(renewed_body(RENEWED_EXP, Some("rotated-token")))]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let found = fixture
        .store
        .fetch("user-1")
        .await
        .expect("the renewal succeeds")
        .expect("the row exists");
    assert_eq!(
        found.id_token.expose_secret(),
        id_token(&claims(RENEWED_EXP))
    );
    assert_eq!(
        found
            .refresh_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("rotated-token")
    );
    assert_eq!(fixture.transport.sent().len(), 1);
    let writes = fixture
        .access
        .writes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(writes.len(), 1);
    let still_stored = fixture
        .inner
        .rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get("user-1")
        .cloned()
        .expect("the inner row stays");
    assert_eq!(
        still_stored.id_token.expose_secret(),
        expired.id_token.expose_secret()
    );
}

#[tokio::test]
async fn a_rejected_renewal_leaves_the_expired_assertion_in_place() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", expired.clone())]),
        transport(vec![Err(rejected(
            "the IdP refused the refresh with status 400",
        ))]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let found = fixture
        .store
        .fetch("user-1")
        .await
        .expect("a refusal is not an outage")
        .expect("the expired row is still returned");
    assert_eq!(
        found.id_token.expose_secret(),
        expired.id_token.expose_secret()
    );
    assert!(
        fixture
            .access
            .writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}

#[tokio::test]
async fn an_unavailable_renewal_fails_the_fetch() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", expired.clone())]),
        transport(vec![Err(outage("the IdP token endpoint is unreachable"))]),
        access_with(vec![("user-1", expired)], AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let failure = fixture
        .store
        .fetch("user-1")
        .await
        .expect_err("an outage fails the fetch");
    assert_eq!(
        failure.to_string(),
        "the SSO identity assertion store is unavailable: the IdP token endpoint is unreachable"
    );
}

#[tokio::test]
async fn a_failing_fresh_read_during_the_renewal_fails_the_fetch() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", expired.clone())]),
        transport(Vec::new()),
        access_with(vec![("user-1", expired)], AccessFailure::Read),
        factory(Vec::new()),
        NOW_MS,
    );
    let failure = fixture
        .store
        .fetch("user-1")
        .await
        .expect_err("the fresh read failed");
    assert_eq!(
        failure.to_string(),
        "the SSO identity assertion store is unavailable: the scripted row read failed"
    );
    assert!(fixture.transport.sent().is_empty());
}

#[tokio::test]
async fn the_inner_store_failure_goes_straight_up() {
    let fixture = StoreFixture::new(
        failing_inner(),
        transport(Vec::new()),
        access_with(Vec::new(), AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let failure = fixture
        .store
        .fetch("user-1")
        .await
        .expect_err("the inner read failed");
    assert_eq!(
        failure.to_string(),
        "the SSO identity assertion store is unavailable: the scripted inner store failed"
    );
    assert!(fixture.transport.sent().is_empty());
}

#[tokio::test]
async fn one_renewal_serves_every_concurrent_fetch() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", expired.clone())]),
        yielding_transport(Ok(renewed_body(RENEWED_EXP, Some("rotated-token")))),
        access_with(vec![("user-1", expired)], AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let store = Arc::new(fixture.store);
    let mut tasks = JoinSet::new();
    let first = Arc::clone(&store);
    tasks.spawn(async move { first.fetch("user-1").await });
    let second = Arc::clone(&store);
    tasks.spawn(async move { second.fetch("user-1").await });
    let third = Arc::clone(&store);
    tasks.spawn(async move { third.fetch("user-1").await });
    let mut found = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let assertion = joined
            .expect("the fetch task did not panic")
            .expect("the fetch succeeds")
            .expect("the row exists");
        found.push(assertion);
    }
    assert_eq!(found.len(), 3);
    let expected = id_token(&claims(RENEWED_EXP));
    assert_eq!(found[0].id_token.expose_secret(), expected.as_str());
    assert_eq!(found[1].id_token.expose_secret(), expected.as_str());
    assert_eq!(found[2].id_token.expose_secret(), expected.as_str());
    assert_eq!(fixture.transport.sent().len(), 1);
    assert_eq!(
        fixture
            .access
            .writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        1
    );
}

#[tokio::test]
async fn a_failed_renewal_reaches_every_concurrent_caller() {
    let exhausted = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", exhausted.clone())]),
        yielding_transport(Err(outage("the IdP token endpoint is unreachable"))),
        access_with(vec![("user-1", exhausted)], AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let store = Arc::new(fixture.store);
    let mut tasks = JoinSet::new();
    let first = Arc::clone(&store);
    tasks.spawn(async move { first.fetch("user-1").await });
    let second = Arc::clone(&store);
    tasks.spawn(async move { second.fetch("user-1").await });
    let third = Arc::clone(&store);
    tasks.spawn(async move { third.fetch("user-1").await });
    let mut messages = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let failure = joined
            .expect("the fetch task did not panic")
            .expect_err("the renewal outage fails every caller");
        messages.push(failure.to_string());
    }
    assert_eq!(messages.len(), 3);
    let expected =
        "the SSO identity assertion store is unavailable: the IdP token endpoint is unreachable";
    assert_eq!(messages[0], expected);
    assert_eq!(messages[1], expected);
    assert_eq!(messages[2], expected);
    assert_eq!(fixture.transport.sent().len(), 1);
}

#[tokio::test]
async fn skips_the_idp_when_the_fresh_row_is_already_renewed() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let current = assertion(RENEWED_EXP, Some("rotated-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", expired)]),
        transport(Vec::new()),
        access_with(vec![("user-1", current.clone())], AccessFailure::None),
        factory(Vec::new()),
        NOW_MS,
    );
    let found = fixture
        .store
        .fetch("user-1")
        .await
        .expect("the fetch succeeds")
        .expect("the row exists");
    assert_eq!(
        found.id_token.expose_secret(),
        current.id_token.expose_secret()
    );
    assert!(fixture.transport.sent().is_empty());
    assert!(
        fixture
            .access
            .writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}

#[tokio::test]
async fn a_loser_whose_reread_still_sees_an_expiring_row_fails_with_the_retry_message() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", expired.clone())]),
        transport(Vec::new()),
        access_with(vec![("user-1", expired)], AccessFailure::None),
        factory(vec![Some(Arc::new(RecordingCoordinator {
            runs: Arc::new(AtomicUsize::new(0)),
            losing: true,
        }))]),
        NOW_MS,
    );
    let failure = fixture
        .store
        .fetch("user-1")
        .await
        .expect_err("the loser retries");
    assert_eq!(
        failure.to_string(),
        "the SSO identity assertion store is unavailable: the IdP identity assertion for user_id=user-1 was being renewed by another replica and is not yet current; retry shortly"
    );
    assert!(fixture.transport.sent().is_empty());
    assert_eq!(fixture.factory.asked.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn a_loser_whose_reread_finds_the_row_current_returns_it() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let current = assertion(RENEWED_EXP, Some("rotated-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", expired)]),
        transport(Vec::new()),
        access_with(vec![("user-1", current.clone())], AccessFailure::None),
        factory(vec![Some(Arc::new(RecordingCoordinator {
            runs: Arc::new(AtomicUsize::new(0)),
            losing: true,
        }))]),
        NOW_MS,
    );
    let found = fixture
        .store
        .fetch("user-1")
        .await
        .expect("the fetched row is current")
        .expect("the row exists");
    assert_eq!(
        found.id_token.expose_secret(),
        current.id_token.expose_secret()
    );
    assert!(fixture.transport.sent().is_empty());
}

async fn fetch_once(fixture: &StoreFixture) {
    fixture
        .store
        .fetch("user-1")
        .await
        .expect("the fetch succeeds")
        .expect("the row exists");
}

#[tokio::test]
async fn keeps_the_distributed_coordinator_once_the_factory_yields_one() {
    let runs = Arc::new(AtomicUsize::new(0));
    let recording: RenewalCoordinator = Arc::new(RecordingCoordinator {
        runs: Arc::clone(&runs),
        losing: false,
    });
    let still_expiring = assertion(NOW_SECONDS + 30, Some("carried-token"));
    let fixture = StoreFixture::new(
        inner(vec![("user-1", still_expiring.clone())]),
        transport(vec![
            Ok(renewed_body(NOW_SECONDS + 30, Some("token-1"))),
            Ok(renewed_body(NOW_SECONDS + 30, Some("token-2"))),
            Ok(renewed_body(NOW_SECONDS + 30, Some("token-3"))),
            Ok(renewed_body(NOW_SECONDS + 30, Some("token-4"))),
        ]),
        access_with(vec![("user-1", still_expiring)], AccessFailure::None),
        factory(vec![None, None, Some(recording)]),
        NOW_MS,
    );
    fetch_once(&fixture).await;
    fetch_once(&fixture).await;
    fetch_once(&fixture).await;
    fetch_once(&fixture).await;
    assert_eq!(fixture.factory.asked.load(Ordering::Relaxed), 3);
    assert_eq!(runs.load(Ordering::Relaxed), 2);
    assert_eq!(fixture.transport.sent().len(), 4);
}

#[test]
fn the_single_flight_key_is_the_sso_assertion_namespace() {
    assert_eq!(SINGLE_FLIGHT_KEY, "sso_identity_assertion");
}

fn db_store() -> DbSsoAssertionStore<MemoryAssertionRows> {
    DbSsoAssertionStore {
        rows: Some(MemoryAssertionRows::default()),
        cipher: StoredCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from("b4664-assertion-key".to_owned())),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        },
        cache: Arc::new(SsoAssertionCache {
            ttl: Duration::from_secs(60),
            max_size: 8,
            clock: Box::new(SystemClock),
            state: Mutex::new(CacheState::default()),
        }),
        config_declares_id_jag: false,
    }
}

#[tokio::test]
async fn the_db_access_reads_none_when_no_row_exists() {
    let store = db_store();
    assert!(
        store
            .read("user-1")
            .await
            .expect("the empty read succeeds")
            .is_none()
    );
}

#[tokio::test]
async fn the_db_access_reads_back_what_it_writes() {
    let store = db_store();
    let stored = assertion(NOW_SECONDS, Some("carried-token"));
    store
        .write("user-1", &stored)
        .await
        .expect("the write succeeds");
    let found = store
        .read("user-1")
        .await
        .expect("the read-back succeeds")
        .expect("the written assertion reads back");
    assert_eq!(
        found.id_token.expose_secret(),
        stored.id_token.expose_secret()
    );
    assert_eq!(
        found
            .refresh_token
            .map(|token| token.expose_secret().to_owned()),
        Some("carried-token".to_owned())
    );
}

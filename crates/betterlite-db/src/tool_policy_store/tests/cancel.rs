use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use deadpool_postgres::Client;
use secrecy::ExposeSecret as _;
use tokio::task::yield_now;
use tokio_postgres::NoTls;

use crate::postgres_store::session::PostgresStore;
use crate::tool_policy_store::deadline::{StatementBounds, TlsCanceller};
use crate::tool_policy_store::discovery::batch_upsert_tools;
use crate::tool_policy_store::errors::ToolPolicyStoreError;
use crate::tool_policy_store::refresh::{ToolPolicyStore, sync_tool_policy_from_db};
use crate::tool_policy_store::registry::ToolPolicyRegistry;
use crate::tool_records::queries::get_tool;

use super::support::{Db, TlsFront, captured_logs};
use super::{FixedIds, Reconnect, instant, item};

const TABLE_LOCK: &str = "BEGIN; LOCK TABLE \"LiteLLM_ToolTable\" IN ACCESS EXCLUSIVE MODE";

async fn locked_tls_front() -> (Db, TlsFront, PostgresStore, Client) {
    let db = Db::start().await;
    let server_url = db.fixture.options.url.expose_secret();
    let front = TlsFront::start(server_url).await;
    let store = front.store(server_url);
    let holder = db.client().await;
    holder
        .batch_execute(TABLE_LOCK)
        .await
        .expect("the table lock");
    (db, front, store, holder)
}

async fn release_table_lock(db: &Db, holder: Client) {
    assert_eq!(wait_until_no_lock_waiters(db).await, 0);
    holder
        .batch_execute("ROLLBACK")
        .await
        .expect("the rollback");
    drop(holder);
}

async fn lock_waiters(db: &Db) -> i64 {
    let client = db.client().await;
    client
        .query_one(
            "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND state = 'active' AND query LIKE '%LiteLLM_ToolTable%'",
            &[],
        )
        .await
        .expect("the database activity")
        .get(0)
}

async fn wait_until_no_lock_waiters(db: &Db) -> i64 {
    let mut waiting = lock_waiters(db).await;
    for _ in 0..100 {
        if waiting == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        waiting = lock_waiters(db).await;
    }
    waiting
}

#[tokio::test]
async fn discovery_stops_at_the_deadline() {
    let db = Db::start().await;
    let holder = db.client().await;
    holder
        .batch_execute(TABLE_LOCK)
        .await
        .expect("the table lock");
    let (logs, guard) = captured_logs();
    batch_upsert_tools(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: Duration::from_millis(100),
            canceller: &TlsCanceller { tls: NoTls },
        },
        &[item(Some("search"))],
        instant(),
    )
    .await;
    drop(guard);
    assert!(
        logs.text()
            .contains("the statement did not finish within 100ms")
    );
    release_table_lock(&db, holder).await;
    let client = db.client().await;
    assert!(get_tool(Some(&client), "search").await.is_none());
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn refresh_cancels_a_read_past_the_deadline() {
    let db = Db::start().await;
    let holder = db.client().await;
    holder
        .batch_execute(TABLE_LOCK)
        .await
        .expect("the table lock");
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let store = ToolPolicyStore {
        database: &db.store,
        registry: &registry,
        bounds: StatementBounds {
            deadline: Duration::from_millis(100),
            canceller: &TlsCanceller { tls: NoTls },
        },
    };
    let mut reconnect = Reconnect::default();
    let result = sync_tool_policy_from_db(&store, Some(&mut reconnect)).await;
    assert!(matches!(result, Err(ToolPolicyStoreError::Deadline { .. })));
    assert!(reconnect.reasons().is_empty());
    assert!(
        !registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .initialized
    );
    release_table_lock(&db, holder).await;
    db.close().await;
}

#[tokio::test]
async fn a_tls_required_cancel_stops_the_read_over_tls() {
    let (db, front, store, holder) = locked_tls_front().await;
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let canceller = front.trusted_connector();
    let policy_store = ToolPolicyStore {
        database: &store,
        registry: &registry,
        bounds: StatementBounds {
            deadline: Duration::from_millis(500),
            canceller: &TlsCanceller { tls: canceller },
        },
    };
    let (logs, guard) = captured_logs();
    let result = sync_tool_policy_from_db(&policy_store, None::<&mut Reconnect>).await;
    drop(guard);
    assert!(matches!(result, Err(ToolPolicyStoreError::Deadline { .. })));
    assert_eq!(
        front.handshakes(),
        2,
        "the session and its cancel request both spoke TLS"
    );
    assert!(!logs.text().contains("could not be cancelled"));
    release_table_lock(&db, holder).await;
    sync_tool_policy_from_db(&policy_store, None::<&mut Reconnect>)
        .await
        .expect("the read after the cancel");
    assert_eq!(
        front.handshakes(),
        2,
        "the cancelled session stayed in the pool"
    );
    store.pool.expect("the TLS pool").close();
    yield_now().await;
    drop(front);
    yield_now().await;
    db.close().await;
}

#[tokio::test]
async fn an_undeliverable_cancel_discards_the_connection() {
    let (db, front, store, holder) = locked_tls_front().await;
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let canceller = TlsFront::untrusted_connector();
    let policy_store = ToolPolicyStore {
        database: &store,
        registry: &registry,
        bounds: StatementBounds {
            deadline: Duration::from_millis(500),
            canceller: &TlsCanceller { tls: canceller },
        },
    };
    let (logs, guard) = captured_logs();
    let result = sync_tool_policy_from_db(&policy_store, None::<&mut Reconnect>).await;
    drop(guard);
    assert!(matches!(result, Err(ToolPolicyStoreError::Deadline { .. })));
    assert!(logs.text().contains("could not be cancelled"));
    let pool = store.pool.as_ref().expect("the TLS pool");
    assert_eq!(
        pool.status().size,
        0,
        "the timed out connection left the pool"
    );
    assert_eq!(pool.status().available, 0);
    release_table_lock(&db, holder).await;
    store.pool.expect("the TLS pool").close();
    yield_now().await;
    drop(front);
    yield_now().await;
    db.close().await;
}

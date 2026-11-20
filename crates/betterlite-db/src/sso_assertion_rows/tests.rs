#![cfg(test)]

use std::sync::Mutex;
use std::sync::atomic::Ordering;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::yield_now;
use tokio_postgres::NoTls;

use super::{
    AssertionRow, AssertionRows, AssertionRowsMethod, MemoryAssertionRows, MemoryFailures,
    MemoryRowsError, PostgresAssertionRows,
};
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::postgres_store::session::PostgresStore;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Db {
    fixture: PostgresFixture,
    rows: PostgresAssertionRows,
}

impl Db {
    async fn start() -> Self {
        let url = Environment::parse_from(["sso-assertion-rows"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            url.as_ref(),
            catalog_statements(&PACKAGED_CATALOGS).unwrap(),
        )
        .resolve()
        .await
        .unwrap();
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let manager = Manager::new(config, NoTls);
        let pool = Pool::builder(manager).max_size(2).build().unwrap();
        Self {
            fixture,
            rows: PostgresAssertionRows {
                store: PostgresStore { pool: Some(pool) },
            },
        }
    }

    async fn client(&self) -> Client {
        self.rows.store.session().await.unwrap()
    }

    async fn close(self) {
        self.rows.store.pool.unwrap().close();
        // The closed pool's drivers need a turn to close their sockets before DROP DATABASE.
        yield_now().await;
        self.fixture.close_async().await.unwrap();
    }
}

// Both timestamps are moved into a fixed past so a statement that never touches one leaves it there.
async fn pin_timestamps(db: &Db, user_id: &str) {
    let client = db.client().await;
    client
        .execute(
            r#"UPDATE "LiteLLM_SSOIdentityAssertion"
            SET created_at = '2001-01-01', updated_at = '2000-01-01'
            WHERE user_id = $1"#,
            &[&user_id],
        )
        .await
        .unwrap();
}

async fn timestamps(db: &Db, user_id: &str) -> (bool, bool) {
    let client = db.client().await;
    let found = client
        .query_one(
            r#"SELECT created_at = '2001-01-01'::timestamp AS created_pinned,
            updated_at > '2020-01-01'::timestamp AS updated_moved
            FROM "LiteLLM_SSOIdentityAssertion" WHERE user_id = $1"#,
            &[&user_id],
        )
        .await
        .unwrap();
    (found.get("created_pinned"), found.get("updated_moved"))
}

fn row(user_id: &str, assertion_b64: &str) -> AssertionRow {
    AssertionRow {
        user_id: user_id.to_owned(),
        assertion_b64: assertion_b64.to_owned(),
    }
}

fn memory_rows(stored: Vec<AssertionRow>) -> MemoryAssertionRows {
    MemoryAssertionRows {
        rows: Mutex::new(stored),
        ..MemoryAssertionRows::default()
    }
}

fn kept_rows(rows: &MemoryAssertionRows) -> Vec<AssertionRow> {
    rows.rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[tokio::test]
async fn upsert_creates_and_replaces_a_row() {
    let db = Db::start().await;
    db.rows.upsert("user-1", "first-ciphertext").await.unwrap();
    assert_eq!(
        db.rows.find("user-1").await.unwrap(),
        Some(row("user-1", "first-ciphertext"))
    );
    db.rows.upsert("user-1", "second-ciphertext").await.unwrap();
    let found = db
        .rows
        .find("user-1")
        .await
        .unwrap()
        .expect("the row stays");
    assert_eq!(found.assertion_b64, "second-ciphertext");
    assert_eq!(
        db.rows.find_all().await.unwrap().len(),
        1,
        "the upsert replaces rather than appends"
    );
    db.close().await;
}

#[tokio::test]
async fn find_and_find_all_return_stored_text() {
    let db = Db::start().await;
    db.rows.upsert("user-b", "bravo").await.unwrap();
    db.rows.upsert("user-a", "alpha").await.unwrap();
    assert_eq!(
        db.rows
            .find("user-a")
            .await
            .unwrap()
            .map(|row| row.assertion_b64),
        Some("alpha".to_owned())
    );
    assert_eq!(db.rows.find("missing").await.unwrap(), None);
    let users: Vec<String> = db
        .rows
        .find_all()
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.user_id)
        .collect();
    assert_eq!(
        users,
        ["user-a", "user-b"],
        "find_all is ordered by user_id"
    );
    db.close().await;
}

#[tokio::test]
async fn update_of_a_missing_row_is_not_an_error() {
    let db = Db::start().await;
    db.rows.update("ghost", "ciphertext").await.unwrap();
    assert_eq!(db.rows.find("ghost").await.unwrap(), None);
    db.close().await;
}

#[tokio::test]
async fn upsert_and_update_move_updated_at() {
    let db = Db::start().await;
    db.rows.upsert("user-1", "first").await.unwrap();
    pin_timestamps(&db, "user-1").await;
    db.rows.upsert("user-1", "second").await.unwrap();
    let (created_pinned, updated_moved) = timestamps(&db, "user-1").await;
    assert!(created_pinned, "the update arm leaves created_at alone");
    assert!(updated_moved, "the update arm stamps updated_at");
    pin_timestamps(&db, "user-1").await;
    db.rows.update("user-1", "third").await.unwrap();
    let (created_after_update, updated_after_update) = timestamps(&db, "user-1").await;
    assert!(created_after_update, "update leaves created_at alone");
    assert!(updated_after_update, "update stamps updated_at");
    let found = db
        .rows
        .find("user-1")
        .await
        .unwrap()
        .expect("the row stays");
    assert_eq!(found.assertion_b64, "third");
    db.close().await;
}

#[tokio::test]
async fn id_jag_server_exists_ignores_every_other_column() {
    let db = Db::start().await;
    assert!(
        !db.rows.id_jag_server_exists().await.unwrap(),
        "no server rows at all"
    );
    let client = db.client().await;
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url, auth_type, status, approval_status)
            VALUES ('plain', 'https://plain.example/mcp', 'oauth2', 'active', 'active'),
                   ('none', 'https://none.example/mcp', NULL, 'active', 'active')"#,
        )
        .await
        .unwrap();
    assert!(
        !db.rows.id_jag_server_exists().await.unwrap(),
        "a plain oauth2 server or a null auth_type does not count"
    );
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url, auth_type, status, approval_status)
            VALUES ('jag', 'https://jag.example/mcp', 'oauth2_id_jag', 'disabled', 'pending')"#,
        )
        .await
        .unwrap();
    assert!(
        db.rows.id_jag_server_exists().await.unwrap(),
        "a disabled, pending id_jag server still turns retention on"
    );
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn memory_rows_keep_insertion_order() {
    let rows = memory_rows(vec![row("user-b", "bravo"), row("user-a", "alpha")]);
    assert_eq!(
        rows.find("user-a").await.unwrap(),
        Some(row("user-a", "alpha"))
    );
    let users: Vec<String> = rows
        .find_all()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.user_id)
        .collect();
    assert_eq!(
        users,
        ["user-b", "user-a"],
        "the fake keeps insertion order"
    );
    rows.upsert("user-a", "replaced").await.unwrap();
    rows.upsert("user-c", "charlie").await.unwrap();
    rows.update("user-b", "bravo-updated").await.unwrap();
    rows.update("ghost", "ignored").await.unwrap();
    let stored: Vec<(String, String)> = kept_rows(&rows)
        .into_iter()
        .map(|entry| (entry.user_id, entry.assertion_b64))
        .collect();
    assert_eq!(
        stored,
        [
            ("user-b".to_owned(), "bravo-updated".to_owned()),
            ("user-a".to_owned(), "replaced".to_owned()),
            ("user-c".to_owned(), "charlie".to_owned()),
        ]
    );
}

#[tokio::test]
async fn memory_rows_count_accepted_calls() {
    let rows = MemoryAssertionRows {
        id_jag: true,
        ..MemoryAssertionRows::default()
    };
    assert!(rows.find("missing").await.unwrap().is_none());
    assert!(rows.find_all().await.unwrap().is_empty());
    assert!(rows.id_jag_server_exists().await.unwrap());
    rows.upsert("user-1", "one").await.unwrap();
    rows.update("user-1", "two").await.unwrap();
    assert_eq!(rows.reads.load(Ordering::SeqCst), 3);
    assert_eq!(rows.writes.load(Ordering::SeqCst), 2);
}

#[rstest]
#[case::find(AssertionRowsMethod::Find)]
#[case::find_all(AssertionRowsMethod::FindAll)]
#[case::upsert(AssertionRowsMethod::Upsert)]
#[case::update(AssertionRowsMethod::Update)]
#[case::id_jag(AssertionRowsMethod::IdJagServerExists)]
#[tokio::test]
async fn memory_rows_fail_per_method(#[case] method: AssertionRowsMethod) {
    let mut failures = MemoryFailures::default();
    match method {
        AssertionRowsMethod::Find => failures.find = Some(0),
        AssertionRowsMethod::FindAll => failures.find_all = Some(0),
        AssertionRowsMethod::Upsert => failures.upsert = Some(0),
        AssertionRowsMethod::Update => failures.update = Some(0),
        AssertionRowsMethod::IdJagServerExists => failures.id_jag_server_exists = Some(0),
    }
    let rows = MemoryAssertionRows {
        failures,
        ..MemoryAssertionRows::default()
    };
    let result = match method {
        AssertionRowsMethod::Find => rows.find("user-1").await.err(),
        AssertionRowsMethod::FindAll => rows.find_all().await.err(),
        AssertionRowsMethod::Upsert => rows.upsert("user-1", "value").await.err(),
        AssertionRowsMethod::Update => rows.update("user-1", "value").await.err(),
        AssertionRowsMethod::IdJagServerExists => rows.id_jag_server_exists().await.err(),
    };
    assert_eq!(result, Some(MemoryRowsError { method }));
    assert_eq!(
        rows.reads.load(Ordering::SeqCst),
        0,
        "a refused call is not a read"
    );
    assert_eq!(
        rows.writes.load(Ordering::SeqCst),
        0,
        "a refused call is not a write"
    );
}

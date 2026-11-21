#![cfg(test)]

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::{FromSql, ToSql};
use tokio_util::task::TaskTracker;

use super::tests::support::{FakeCache, FakeEnrollment, FakeReset, request_with, unavailable};
use crate::auth_user_lookup::cache::UserCache;
use crate::auth_user_lookup::cadence::{Cadence, WallClock};
use crate::auth_user_lookup::error::{UserLookupError, UserReadFailure};
use crate::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use crate::auth_user_lookup::ports::{BudgetResetCalculator, DefaultTeamEnrollment};
use crate::auth_user_lookup::rows::UserRowError;
use crate::auth_user_lookup::store::{DatabaseUserStore, UserStore};

const SEED_CASE_EMAIL: &str = "INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email) VALUES ('case-user', 'MixedCase@Example.com'), ('other-user', 'other@example.com')";
const SEED_BACKFILL: &str = "INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email) VALUES ('filled-user', 'stored@example.com'), ('empty-user', ''), ('null-user', NULL)";
const SEED_SSO: &str = "INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email, sso_user_id, updated_at) VALUES ('sso-user', 'sso@example.com', 'old-sso', TIMESTAMP '2000-01-01 00:00:00')";
const SEED_CAP: &str = "INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email, user_alias) VALUES ('cap-user', 'cap@example.com', 'cap')";
const ROW_CAP_BYTES: i64 = 0x0010_0000;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn provision() -> PostgresFixture {
    let url = Environment::parse_from(["auth-user-lookup-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture")
}

fn pool_for(fixture: &PostgresFixture) -> Pool {
    let manager = Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    );
    Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("the database pool")
}

fn tracker() -> Arc<DbLookupStallTracker<SystemClock>> {
    Arc::new(DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    })
}

async fn database() -> (PostgresFixture, Arc<DatabaseUserStore>) {
    let fixture = provision().await;
    let store = PostgresStore {
        pool: Some(pool_for(&fixture)),
    };
    let database = DatabaseUserStore {
        reader: store.clone(),
        writer: store,
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0,
        tracker: tracker(),
        tasks: TaskTracker::new(),
    };
    (fixture, Arc::new(database))
}

async fn close(fixture: PostgresFixture, store: Arc<DatabaseUserStore>) {
    store.tasks.close();
    store.tasks.wait().await;
    if let Some(pool) = store.writer.pool.as_ref() {
        pool.close();
    }
    fixture.close_async().await.expect("the fixture teardown");
}

fn lookup_for(store: &Arc<DatabaseUserStore>, cache: Arc<FakeCache>) -> UserLookup {
    let cache_port: Arc<dyn UserCache> = cache;
    let store_port: Arc<dyn UserStore> = Arc::<DatabaseUserStore>::clone(store);
    let reset: Arc<dyn BudgetResetCalculator> = Arc::new(FakeReset::default());
    let enrollment: Arc<dyn DefaultTeamEnrollment> = Arc::new(FakeEnrollment::default());
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the built-in patterns compile");
    UserLookup {
        cache: cache_port,
        store: Some(store_port),
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: reset,
        enrollment,
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(redactor),
    }
}

async fn seed(store: &PostgresStore, sql: &str) {
    let client = store.session().await.expect("the database connection");
    client.batch_execute(sql).await.expect("the seeded rows");
}

async fn scalar<T>(store: &PostgresStore, sql: &str, params: &[&(dyn ToSql + Sync)]) -> T
where
    T: for<'row> FromSql<'row>,
{
    let client = store.session().await.expect("the database connection");
    client
        .query_one(sql, params)
        .await
        .expect("the queried row")
        .try_get(0)
        .expect("the queried column")
}

async fn stored_column(store: &PostgresStore, column: &str, user_id: &str) -> Option<String> {
    let sql = format!("SELECT {column} FROM \"LiteLLM_UserTable\" WHERE user_id = $1");
    scalar(store, &sql, &[&user_id]).await
}

async fn stored_email(store: &PostgresStore, user_id: &str) -> Option<String> {
    stored_column(store, "user_email", user_id).await
}

async fn stored_sso(store: &PostgresStore, user_id: &str) -> Option<String> {
    stored_column(store, "sso_user_id", user_id).await
}

async fn stored_updated_after(store: &PostgresStore, user_id: &str) -> bool {
    let sql =
        "SELECT updated_at > TIMESTAMP '2000-01-02' FROM \"LiteLLM_UserTable\" WHERE user_id = $1";
    scalar(store, sql, &[&user_id]).await
}

async fn row_text_len(store: &PostgresStore, user_id: &str) -> i64 {
    let sql = "SELECT length((to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text)::bigint FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_id\" = $1";
    scalar(store, sql, &[&user_id]).await
}

async fn grow_alias_to(store: &PostgresStore, user_id: &str, target: i64) {
    let length = row_text_len(store, user_id).await;
    let delta = target
        .checked_sub(length)
        .expect("the seeded row starts below the target");
    let repeat_count = i32::try_from(delta).expect("the growth fits a repeat count");
    let sql = "UPDATE \"LiteLLM_UserTable\" SET user_alias = COALESCE(user_alias, '') || repeat('a', $2) WHERE user_id = $1";
    let client = store.session().await.expect("the database connection");
    client
        .execute(sql, &[&user_id, &repeat_count])
        .await
        .expect("the row growth");
}

async fn user_count(store: &PostgresStore, user_id: &str) -> i64 {
    let sql = "SELECT count(*) FROM \"LiteLLM_UserTable\" WHERE user_id = $1";
    scalar(store, sql, &[&user_id]).await
}

async fn lookup_email(store: &Arc<DatabaseUserStore>, user_id: &str) -> Option<String> {
    let lookup = lookup_for(store, Arc::new(FakeCache::new(60.0)));
    let user = lookup
        .get_user_object(request_with(Some(user_id), None, Some("new@example.com")))
        .await
        .expect("the stored user")
        .expect("the stored user is present");
    user.user_email
}

fn names_row_cap(failure: &UserReadFailure) -> bool {
    let mut names_cap = false;
    let mut link = failure.cause.source();
    while let Some(current) = link {
        names_cap |= current.to_string().contains("over the");
        link = current.source();
    }
    names_cap
}

#[tokio::test]
async fn email_case_insensitive_first() {
    let (fixture, store) = database().await;
    seed(&store.reader, SEED_CASE_EMAIL).await;
    let lookup = lookup_for(&store, Arc::new(FakeCache::new(60.0)));
    let user = lookup
        .get_user_object(request_with(
            Some("requested-case"),
            None,
            Some("mixedcase@example.com"),
        ))
        .await
        .expect("the fuzzy email lookup")
        .expect("the matched user");
    assert_eq!(user.user_id, "case-user");
    assert_eq!(user.user_email.as_deref(), Some("MixedCase@Example.com"));
    close(fixture, store).await;
}

#[tokio::test]
async fn backfill_only_null_email() {
    let (fixture, store) = database().await;
    seed(&store.reader, SEED_BACKFILL).await;

    let filled = lookup_email(&store, "filled-user").await;
    assert_eq!(filled.as_deref(), Some("stored@example.com"));
    let filled_stored = stored_email(&store.reader, "filled-user").await;
    assert_eq!(filled_stored.as_deref(), Some("stored@example.com"));

    let empty = lookup_email(&store, "empty-user").await;
    assert_eq!(empty.as_deref(), Some(""));
    let empty_stored = stored_email(&store.reader, "empty-user").await;
    assert_eq!(empty_stored.as_deref(), Some(""));

    let null = lookup_email(&store, "null-user").await;
    assert_eq!(null.as_deref(), Some("new@example.com"));
    let null_stored = stored_email(&store.reader, "null-user").await;
    assert_eq!(null_stored.as_deref(), Some("new@example.com"));
    close(fixture, store).await;
}

#[tokio::test]
async fn sso_update_overwrite_reaches_rows() {
    let (fixture, store) = database().await;
    seed(&store.reader, SEED_SSO).await;
    let lookup = lookup_for(&store, Arc::new(FakeCache::new(60.0)));
    let user = lookup
        .get_user_object(request_with(
            Some("wanted"),
            Some("new-sso"),
            Some("sso@example.com"),
        ))
        .await
        .expect("the fuzzy lookup")
        .expect("the matched user");
    assert_eq!(user.user_id, "sso-user");
    lookup.tasks.close();
    lookup.tasks.wait().await;
    assert_eq!(
        stored_sso(&store.reader, "sso-user").await.as_deref(),
        Some("new-sso")
    );
    assert!(
        stored_updated_after(&store.reader, "sso-user").await,
        "the update stamps updated_at like Prisma's @updatedAt"
    );
    close(fixture, store).await;
}

#[tokio::test]
async fn row_cap_boundary() {
    let (fixture, store) = database().await;
    seed(&store.reader, SEED_CAP).await;
    grow_alias_to(&store.writer, "cap-user", ROW_CAP_BYTES).await;
    assert_eq!(row_text_len(&store.reader, "cap-user").await, ROW_CAP_BYTES);
    let lookup = lookup_for(&store, Arc::new(FakeCache::new(60.0)));
    let user = lookup
        .get_user_object(request_with(Some("cap-user"), None, None))
        .await
        .expect("the row at the cap decodes")
        .expect("the capped user");
    assert_eq!(user.user_id, "cap-user");

    grow_alias_to(&store.writer, "cap-user", ROW_CAP_BYTES + 1).await;
    let over = lookup_for(&store, Arc::new(FakeCache::new(60.0)));
    let result = over
        .get_user_object(request_with(Some("cap-user"), None, None))
        .await;
    match result {
        Err(UserLookupError::UserMissing(failure)) => {
            assert!(
                names_row_cap(&failure),
                "the failure names the row cap: {failure}"
            );
        }
        other => panic!("a row over the cap fails the lookup: {other:?}"),
    }
    close(fixture, store).await;
}

#[tokio::test]
async fn created_row_and_memberships_persist() {
    let (fixture, store) = database().await;
    let lookup = lookup_for(&store, Arc::new(FakeCache::new(60.0)));
    let user = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("fresh-user"),
            sso_user_id: None,
            user_email: Some("fresh@example.com"),
            user_id_upsert: true,
            check_db_only: None,
        })
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(user.user_id, "fresh-user");
    assert_eq!(user.user_email.as_deref(), Some("fresh@example.com"));
    assert_eq!(user.organization_memberships, Some(Vec::new()));
    assert_eq!(user_count(&store.reader, "fresh-user").await, 1);
    assert_eq!(
        stored_email(&store.reader, "fresh-user").await.as_deref(),
        Some("fresh@example.com")
    );
    close(fixture, store).await;
}

#[tokio::test]
async fn partial_failure_keeps_created_row() {
    let (fixture, store) = database().await;
    let cache = Arc::new(FakeCache::new(60.0));
    cache.write_fails.store(true, Ordering::SeqCst);
    let lookup = lookup_for(&store, cache);
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("partial-user"),
            sso_user_id: None,
            user_email: Some("partial@example.com"),
            user_id_upsert: true,
            check_db_only: None,
        })
        .await;
    match result {
        Err(_) => {}
        Ok(value) => panic!("the cache write failure fails the lookup: {value:?}"),
    }
    assert_eq!(user_count(&store.reader, "partial-user").await, 1);
    close(fixture, store).await;
}

#[tokio::test]
async fn writer_flag_reads_through_the_writer() {
    let fixture = provision().await;
    let writer = PostgresStore {
        pool: Some(pool_for(&fixture)),
    };
    let store = Arc::new(DatabaseUserStore {
        reader: PostgresStore { pool: None },
        writer: writer.clone(),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0,
        tracker: tracker(),
        tasks: TaskTracker::new(),
    });
    seed(&writer, SEED_CASE_EMAIL).await;
    let found = store
        .find_by_id("case-user", true)
        .await
        .expect("the writer lookup")
        .expect("the seeded user");
    match found {
        JsonData::Object(fields) => assert_eq!(
            fields.get("user_id"),
            Some(&JsonData::String("case-user".to_owned()))
        ),
        other => panic!("the writer returns the stored user row: {other:?}"),
    }
    close(fixture, store).await;
}

#[tokio::test]
async fn stale_plan_driver_errors_are_unavailable() {
    let (fixture, store) = database().await;
    seed(&store.reader, SEED_CAP).await;
    let client = store
        .reader
        .session()
        .await
        .expect("the database connection");
    let statement = client
        .prepare("SELECT * FROM \"LiteLLM_UserTable\" WHERE user_id = $1")
        .await
        .expect("the prepared statement");
    client
        .query(&statement, &[&"cap-user"])
        .await
        .expect("the first plan");
    client
        .batch_execute("ALTER TABLE \"LiteLLM_UserTable\" ADD COLUMN b736_plan_marker integer")
        .await
        .expect("the schema change");
    let driver = client
        .query(&statement, &[&"cap-user"])
        .await
        .expect_err("the changed result type invalidates the cached plan");
    assert!(
        driver.as_db_error().is_some_and(|error| error
            .message()
            .contains("cached plan must not change result type")),
        "the driver carries the stale plan: {driver}"
    );
    let native = backend::<UserRowError>(driver);
    assert!(unavailable(Box::new(native)));
    drop(client);
    close(fixture, store).await;
}

#[tokio::test]
async fn driver_data_errors_are_missing() {
    let (fixture, store) = database().await;
    let client = store
        .reader
        .session()
        .await
        .expect("the database connection");
    let driver = client
        .query("SELECT * FROM \"B736MissingTable\"", &[])
        .await
        .expect_err("the table does not exist");
    let native = backend::<UserRowError>(driver);
    assert!(!unavailable(Box::new(native)));
    drop(client);
    close(fixture, store).await;
}

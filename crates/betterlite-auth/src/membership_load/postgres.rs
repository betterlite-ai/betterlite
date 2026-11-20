#![cfg(test)]

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::membership::{Counter, MembershipBudget};
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::error::FixtureError;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use super::rows::{DatabaseTeamMembership, TeamMembershipRows};
use super::tests::support::{
    FakeCache, RecordedWrite, WriteKind, drain, expect_absent, index, key,
};
use crate::membership_load::MembershipLookup;

const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, tpm_limit, rpm_limit, created_by, updated_by)
    VALUES ('budget-1', 500.0, 11, 7, 'tester', 'tester');
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend, budget_id)
    VALUES ('user-1', 'team-1', 12.5, 'budget-1'),
           ('user-1', 'team-2', 1.0, NULL),
           ('user-2', 'team-1', 2.0, NULL);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["membership-load-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let pool = Pool::builder(Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    ))
    .max_size(2)
    .build()
    .expect("the database pool");
    (fixture, pool)
}

async fn seed(pool: &Pool) {
    let client = pool.get().await.expect("the database connection");
    client.batch_execute(SEED).await.expect("the seeded rows");
}

async fn seed_spend(pool: &Pool, spend: f64) {
    let client = pool.get().await.expect("the database connection");
    client
        .execute(
            r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend)
               VALUES ('user-1', 'team-1', $1)"#,
            &[&spend],
        )
        .await
        .expect("the membership row");
}

async fn snapshot(pool: &Pool) -> (i64, f64) {
    let client = pool.get().await.expect("the database connection");
    let row = client
        .query_one(
            r#"SELECT count(*), coalesce(max(spend), 0.0) FROM "LiteLLM_TeamMembership""#,
            &[],
        )
        .await
        .expect("the membership snapshot");
    (row.get(0), row.get(1))
}

fn database_rows_with(
    writer: &Pool,
    reader: &Pool,
    availability: Arc<Availability>,
) -> DatabaseTeamMembership {
    DatabaseTeamMembership {
        writer: Arc::new(PostgresStore {
            pool: Some(writer.clone()),
        }),
        reader: Arc::new(PostgresStore {
            pool: Some(reader.clone()),
        }),
        availability,
        default_deadline_seconds: 5.0_f64,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }
}

fn database_rows(pool: &Pool) -> DatabaseTeamMembership {
    database_rows_with(pool, pool, Arc::new(Availability::default()))
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

#[tokio::test]
async fn membership_with_budget() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    let rows = database_rows(&pool);
    let found = rows
        .find_membership("user-1", "team-1")
        .await
        .expect("the membership read")
        .expect("the stored membership");
    assert_eq!(found.budget_id.as_deref(), Some("budget-1"));
    assert_eq!(found.spend, Counter(Some(Decimal::new(125, 1))));
    assert_eq!(found.safe_get_team_member_rpm_limit(), Some(7_i64));
    assert_eq!(found.safe_get_team_member_tpm_limit(), Some(11_i64));
    let budget = found.litellm_budget_table.expect("the joined budget");
    let max_budget = match budget {
        MembershipBudget::Full(row) => row.budget.max_budget,
        MembershipBudget::Base(row) => row.max_budget,
    };
    assert_eq!(max_budget, Some(Decimal::new(500, 0)));
    let neighbour = rows
        .find_membership("user-1", "team-2")
        .await
        .expect("the neighbour read")
        .expect("the neighbour row");
    assert_eq!(neighbour.spend, Counter(Some(Decimal::ONE)));
    let other = rows
        .find_membership("user-2", "team-1")
        .await
        .expect("the other read")
        .expect("the other row");
    assert_eq!(other.spend, Counter(Some(Decimal::new(2, 0))));
    assert!(
        rows.find_membership("user-9", "team-9")
            .await
            .expect("the missing read")
            .is_none()
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn absent_membership_cached() {
    let (fixture, pool) = database().await;
    let cache = FakeCache::new(77.5_f64);
    let tasks = TaskTracker::new();
    let rows: Arc<dyn TeamMembershipRows> = Arc::new(database_rows(&pool));
    let subject = MembershipLookup {
        cache: Arc::clone(&cache),
        rows: Some(rows),
        index: index().1,
        tasks: tasks.clone(),
    };
    let found = subject
        .get_team_membership(Some("user-1"), Some("team-1"))
        .await;
    expect_absent(found);
    assert_eq!(
        cache.writes(),
        vec![RecordedWrite {
            key: key("user-1", "team-1"),
            kind: WriteKind::Absent,
            ttl: Some(77.5_f64),
        }]
    );
    drain(&tasks).await;
    close(fixture, pool).await;
}

#[tokio::test]
async fn lookup_does_not_mutate() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    let before = snapshot(&pool).await;
    let rows = database_rows(&pool);
    let present = rows
        .find_membership("user-1", "team-1")
        .await
        .expect("the present read");
    let absent = rows
        .find_membership("user-9", "team-9")
        .await
        .expect("the absent read");
    assert!(present.is_some());
    assert!(absent.is_none());
    assert_eq!(snapshot(&pool).await, before);
    close(fixture, pool).await;
}

#[tokio::test]
async fn normal_read_routing() {
    let (reader_fixture, reader_pool) = database().await;
    let (writer_fixture, writer_pool) = database().await;
    seed_spend(&reader_pool, 1.5_f64).await;
    seed_spend(&writer_pool, 2.5_f64).await;
    let availability = Arc::new(Availability::default());
    let rows = database_rows_with(&writer_pool, &reader_pool, Arc::clone(&availability));
    let from_reader = rows
        .find_membership("user-1", "team-1")
        .await
        .expect("the reader read")
        .expect("the reader row");
    assert_eq!(from_reader.spend, Counter(Some(Decimal::new(15, 1))));
    availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let from_writer = rows
        .find_membership("user-1", "team-1")
        .await
        .expect("the writer read")
        .expect("the writer row");
    assert_eq!(from_writer.spend, Counter(Some(Decimal::new(25, 1))));
    close(reader_fixture, reader_pool).await;
    close(writer_fixture, writer_pool).await;
}

#[tokio::test]
// ast-grep-ignore: no-long-names
async fn provided_database_url_failure_is_fatal() {
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let broken = SecretString::from("postgres://localhost:not-a-port/betterlite");
    let failure = PostgresFixture::provision_with_statements(Some(&broken), statements)
        .resolve()
        .await
        .expect_err("a provided broken URL must not fall back to a container");
    assert!(
        matches!(failure, FixtureError::InvalidConfiguredUrl { .. }),
        "{failure}"
    );
}

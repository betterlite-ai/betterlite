#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::SecretString;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use super::super::cache::ValidationCache;
use super::super::existence::{EndUserExistence, EndUserObjectExistence, UserExistence};
use super::super::resolve::{DatabaseLookups, EndUserIdResolver, ResolveRequest};
use super::support::{TestCache, lookup_over, registry, test_cache};
use crate::auth_end_users::postgres::DatabaseEndUsers;
use crate::auth_user_lookup::cache::UserCache;
use crate::auth_user_lookup::lookup::UserLookup;
use crate::auth_user_lookup::store::{DatabaseUserStore, UserStore};

const SEED: &str = r#"
INSERT INTO "LiteLLM_EndUserTable" (user_id, spend) VALUES ('known', 0.0);
INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('known-user');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["auth-end-user-identity-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
        .max_size(2)
        .build()
        .expect("the database pool");
    (fixture, pool)
}

fn store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

fn tracker() -> Arc<DbLookupStallTracker<SystemClock>> {
    Arc::new(DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    })
}

fn end_user_rows(pool: &Pool) -> DatabaseEndUsers {
    DatabaseEndUsers {
        writer: Arc::new(store(pool)),
        reader: Arc::new(store(pool)),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: tracker(),
        tasks: TaskTracker::new(),
    }
}

fn user_store(pool: &Pool) -> DatabaseUserStore {
    DatabaseUserStore {
        reader: store(pool),
        writer: store(pool),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: tracker(),
        tasks: TaskTracker::new(),
    }
}

async fn seed(pool: &Pool) {
    let client = store(pool)
        .session()
        .await
        .expect("the database connection");
    client.batch_execute(SEED).await.expect("the seeded rows");
}

async fn count(pool: &Pool, table: &str) -> i64 {
    let client = store(pool)
        .session()
        .await
        .expect("the database connection");
    let sql = format!(r#"SELECT count(*) FROM "{table}""#);
    let row = client
        .query_one(&sql, &[])
        .await
        .expect("the counted table");
    row.try_get(0).expect("the count column")
}

fn request(end_user_id: &str) -> ResolveRequest<'_> {
    ResolveRequest {
        raw_end_user_id: Some(end_user_id),
        key_end_user_budget_id: None,
    }
}

async fn resolve(identity: &EndUserIdResolver, end_user_id: &str) -> Option<String> {
    identity
        .resolve(&request(end_user_id))
        .await
        .expect("the lookup")
}

#[tokio::test]
async fn postgres_validation_reads_the_real_tables() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    let rows = Arc::new(end_user_rows(&pool));
    let cache = Arc::new(test_cache());
    let existence = EndUserObjectExistence {
        rows: Some(Arc::<DatabaseEndUsers>::clone(&rows)),
        budgets: Some(Arc::<DatabaseEndUsers>::clone(&rows)),
        cache: Arc::clone(&cache),
        registry: Arc::new(registry(true, None)),
        store: store(&pool),
        max_end_user_budget_id: None,
    };
    let end_users: Arc<dyn EndUserExistence> = Arc::new(existence);
    let users_store = Arc::new(user_store(&pool));
    let user_cache: Arc<dyn UserCache> = Arc::clone(&cache);
    let user_ports: Arc<dyn UserStore> = Arc::<DatabaseUserStore>::clone(&users_store);
    let lookup = Arc::new(lookup_over(user_cache, user_ports));
    let users: Arc<dyn UserExistence> = Arc::<UserLookup>::clone(&lookup);
    let validation: Arc<dyn ValidationCache> = Arc::clone(&cache);
    let identity = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
        lookups: Some(DatabaseLookups { end_users, users }),
        cache: validation,
    };
    assert_eq!(resolve(&identity, "known").await.as_deref(), Some("known"));
    assert_eq!(
        resolve(&identity, "known-user").await.as_deref(),
        Some("known-user")
    );
    assert!(resolve(&identity, "ghost").await.is_none());
    assert_eq!(count(&pool, "LiteLLM_UserTable").await, 1);
    assert_eq!(count(&pool, "LiteLLM_EndUserTable").await, 1);
    lookup.tasks.close();
    lookup.tasks.wait().await;
    users_store.tasks.close();
    users_store.tasks.wait().await;
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

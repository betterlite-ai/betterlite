#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_cache::coordinator::EventDrivenCacheCoordinator;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use super::global_spend::{Alerts, FakeCache};
use super::support::Harness;
use crate::auth_model_budgets::checks::read_user_model_max_budget;
use crate::auth_model_budgets::global_spend::{GlobalSpendFetcher, get_global_proxy_spend};
use crate::auth_user_lookup::store::DatabaseUserStore;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["model-budget-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).unwrap();
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .unwrap();
    let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
        .max_size(4)
        .build()
        .unwrap();
    (fixture, pool)
}

fn spend_fetcher(store: &PostgresStore, alerts: &Arc<Alerts>) -> GlobalSpendFetcher {
    GlobalSpendFetcher {
        rows: Arc::new(store.clone()),
        cache: Arc::new(FakeCache::default()),
        coordinator: EventDrivenCacheCoordinator::new(
            "[GLOBAL SPEND]".to_owned(),
            Duration::from_secs(5),
        ),
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
        default_deadline_seconds: 5.0,
        alerts: Arc::<Alerts>::clone(alerts),
    }
}

async fn check_spend(fetcher: &GlobalSpendFetcher, store: &PostgresStore, alerts: &Alerts) {
    assert_eq!(
        get_global_proxy_spend(Decimal::ONE, Some(fetcher), "admin", "token")
            .await
            .unwrap(),
        None
    );
    assert!(alerts.scheduled.lock().unwrap().is_empty());
    let client = store.session().await.unwrap();
    client.batch_execute("INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, model_max_budget) VALUES ('litellm-proxy-budget', 0, '{}'), ('u1', 0, '{\"model\":3}')").await.unwrap();
    assert_eq!(
        get_global_proxy_spend(Decimal::ONE, Some(fetcher), "admin", "token")
            .await
            .unwrap(),
        Some(Decimal::ZERO)
    );
    assert_eq!(alerts.scheduled.lock().unwrap().len(), 1);
}

async fn check_users(store: &PostgresStore) {
    let mut lookup = Harness::default().lookup();
    lookup.store = Some(Arc::new(DatabaseUserStore {
        reader: store.clone(),
        writer: store.clone(),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }));
    assert_eq!(
        read_user_model_max_budget(Some(&lookup), Some("missing")).await,
        None
    );
    let client = store.session().await.unwrap();
    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM \"LiteLLM_UserTable\" WHERE user_id = 'missing'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
    let budget = read_user_model_max_budget(Some(&lookup), Some("u1"))
        .await
        .unwrap();
    assert_eq!(serde_json::to_string(&budget).unwrap(), r#"{"model":3}"#);
    lookup.tasks.close();
    lookup.tasks.wait().await;
}

#[tokio::test]
async fn postgres_spend_missing_zero_and_user_no_upsert() {
    let (fixture, pool) = database().await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let alerts = Arc::new(Alerts::default());
    let fetcher = spend_fetcher(&store, &alerts);
    check_spend(&fetcher, &store, &alerts).await;
    check_users(&store).await;
    fetcher.tasks.close();
    fetcher.tasks.wait().await;
    drop(fetcher);
    drop(store);
    pool.close();
    fixture.close_async().await.unwrap();
}

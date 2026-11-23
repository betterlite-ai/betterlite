use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::auth_org_lookup::error::{OrgFailure, OrgLookupError, is_outage};
use crate::auth_org_lookup::ports::OrgStore as _;
use crate::auth_org_lookup::store::DatabaseOrgStore;
use crate::auth_prefetch::rows::CachedProject;

const SEED: &str = "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id, max_budget, allowed_models, created_by, updated_by) VALUES ('b1', 10.5, ARRAY['gpt-4o']::TEXT[], 'a', 'a'); INSERT INTO \"LiteLLM_OrganizationTable\" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'acme', 'b1', 'a', 'a'), ('o2', 'shared', 'b1', 'a', 'a'), ('o3', 'shared', 'b1', 'a', 'a'); INSERT INTO \"LiteLLM_ProjectTable\" (project_id, project_alias, budget_id, created_by, updated_by) VALUES ('p1', 'proj', 'b1', 'a', 'a')";

// Postgres has no SELECT trigger, so the org table becomes a view whose predicate raises the server's
// stale-plan message, the one driver error that only the message tells apart from an ordinary fault
const STALE_PLAN_SETUP: &str = "
ALTER TABLE \"LiteLLM_OrganizationTable\" RENAME TO \"LiteLLM_OrganizationTable_base\";
CREATE FUNCTION raise_select_error() RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'cached plan must not change result type';
END
$$;
CREATE VIEW \"LiteLLM_OrganizationTable\" AS
    SELECT * FROM \"LiteLLM_OrganizationTable_base\" WHERE raise_select_error()";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn provision() -> PostgresFixture {
    let url = Environment::parse_from(["auth-org-lookup-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).unwrap();
    PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .unwrap()
}

fn pool_for(fixture: &PostgresFixture) -> Pool {
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    Pool::builder(manager).max_size(4).build().unwrap()
}

fn org_store(reader: PostgresStore, writer: PostgresStore) -> DatabaseOrgStore {
    DatabaseOrgStore {
        reader,
        writer,
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }
}

async fn assert_orgs(database: &DatabaseOrgStore) {
    let with_budget: OrganizationTable =
        serde_json::from_str(&database.find_org("o1", true).await.unwrap().unwrap()).unwrap();
    assert_eq!(with_budget.organization_id.as_deref(), Some("o1"));
    assert!(with_budget.models.is_empty(), "a null text[] reads as []");
    let budget = with_budget.litellm_budget_table.unwrap();
    assert_eq!(budget.allowed_models, Some(vec!["gpt-4o".to_owned()]));
    assert!(budget.max_budget.is_some());
    let plain: OrganizationTable =
        serde_json::from_str(&database.find_org("o1", false).await.unwrap().unwrap()).unwrap();
    assert!(plain.litellm_budget_table.is_none());
    assert_eq!(database.find_org("absent", true).await.unwrap(), None);
    let shared = database.find_by_alias("shared").await.unwrap();
    assert_eq!(shared.len(), 2);
    for row in &shared {
        let member: OrganizationTable = serde_json::from_str(row).unwrap();
        assert!(member.litellm_budget_table.is_none());
    }
    assert_eq!(database.find_by_alias("acme").await.unwrap().len(), 1);
    assert!(database.find_by_alias("nobody").await.unwrap().is_empty());
}

async fn assert_projects(database: &DatabaseOrgStore) {
    let text = database.find_project("p1").await.unwrap().unwrap();
    let project: CachedProject = serde_json::from_str(&text).unwrap();
    assert_eq!(project.project.project_id, "p1");
    assert_eq!(project.project.project_alias.as_deref(), Some("proj"));
    assert!(project.project.litellm_budget_table.is_some());
    assert_eq!(project.last_refreshed_at, None);
    assert_eq!(database.find_project("absent").await.unwrap(), None);
}

#[tokio::test]
async fn relation_projection() {
    let fixture = provision().await;
    let store = PostgresStore {
        pool: Some(pool_for(&fixture)),
    };
    store
        .session()
        .await
        .unwrap()
        .batch_execute(SEED)
        .await
        .unwrap();
    let database = org_store(store.clone(), store.clone());
    assert_orgs(&database).await;
    assert_projects(&database).await;

    let writer_only = org_store(PostgresStore { pool: None }, store.clone());
    assert!(
        writer_only.find_org("o1", false).await.is_err(),
        "the reader handle is not connected"
    );
    writer_only
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    assert!(writer_only.find_org("o1", false).await.unwrap().is_some());

    store
        .session()
        .await
        .unwrap()
        .batch_execute(STALE_PLAN_SETUP)
        .await
        .unwrap();
    let stale = database.find_org("o1", false).await.unwrap_err();
    assert!(
        is_outage(&OrgLookupError::Failure(OrgFailure::Store(stale))),
        "a stale plan on the replica is an outage, not a missing row"
    );

    database.tasks.close();
    database.tasks.wait().await;
    writer_only.tasks.close();
    writer_only.tasks.wait().await;
    if let Some(pool) = store.pool.as_ref() {
        pool.close();
    }
    fixture.close_async().await.unwrap();
}

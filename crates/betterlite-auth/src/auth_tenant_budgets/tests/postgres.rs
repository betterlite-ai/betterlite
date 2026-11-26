use std::sync::{Arc, Mutex};

use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_org_lookup::ports::{OrgCache, OrgStore};
use crate::auth_org_lookup::store::DatabaseOrgStore;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::table::ReadContext;
use crate::auth_resource_grants::postgres::open_grant_rows;
use crate::auth_tag_budgets::cache::TagBudgetCache;
use crate::auth_tag_budgets::rows::group::ModelAccessGroupBudgetTable;
use crate::auth_tag_budgets::rows::tag::TagBudgetTable;
use crate::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow};
use crate::auth_tenant_budgets::alerts::TenantAlertKind;
use crate::auth_tenant_budgets::error::TenantBudgetError;
use crate::auth_tenant_budgets::{TenantAdmission, TenantRequest, TenantSources, admit};

use super::support::{OrgWorld, Recorder, ScriptedSpend, TagWorld, decimal, number, team, token};

// every entity gets its own capped budget row, so a denial names the entity that was actually exhausted
const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" ("budget_id", "max_budget", "created_by", "updated_by")
    VALUES ('project-cap', 10, 'seed', 'seed'),
           ('org-cap', 20, 'seed', 'seed'),
           ('tag-cap', 5, 'seed', 'seed'),
           ('group-cap', 4, 'seed', 'seed'),
           ('roomy-cap', 100, 'seed', 'seed'),
           ('no-ceiling', NULL, 'seed', 'seed');
INSERT INTO "LiteLLM_ProjectTable"
        ("project_id", "project_alias", "budget_id", "spend", "created_by", "updated_by")
    VALUES ('p-spent', 'spent-project', 'project-cap', 0, 'seed', 'seed'),
           ('p-plain', 'plain-project', NULL, 0, 'seed', 'seed');
INSERT INTO "LiteLLM_OrganizationTable"
        ("organization_id", "organization_alias", "budget_id", "spend", "created_by", "updated_by")
    VALUES ('o-spent', 'spent-org', 'org-cap', 2, 'seed', 'seed'),
           ('o-unpriced', 'unpriced-org', 'no-ceiling', 0, 'seed', 'seed');
INSERT INTO "LiteLLM_TagTable" ("tag_name", "models", "spend", "budget_id")
    VALUES ('tag-spent', ARRAY['m1'], 1, 'tag-cap'),
           ('tag-roomy', ARRAY['m1'], 0, 'roomy-cap'),
           ('tag-unpriced', ARRAY[]::text[], 3, 'no-ceiling');
INSERT INTO "LiteLLM_ModelAccessGroupBudgetTable" ("access_group_name", "spend", "budget_id")
    VALUES ('group-spent', 1, 'group-cap'),
           ('group-roomy', 0, 'roomy-cap');
"#;

struct Admitted {
    fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
    database: Arc<DatabaseOrgStore>,
    lookup: OrgLookup,
    cache: TagWorld,
    tags: Box<dyn BudgetRows<TagTable>>,
    groups: Box<dyn BudgetRows<ModelAccessGroupBudgetRow>>,
}

impl Admitted {
    async fn provision() -> Self {
        let (fixture, pool) = database().await;
        let store = PostgresStore {
            pool: Some(pool.clone()),
        };
        let (database, lookup) = org_store(store.clone());
        let (tags, groups) = grant_rows(&pool).await;
        Self {
            fixture,
            pool,
            store,
            database,
            lookup,
            cache: TagWorld::default(),
            tags,
            groups,
        }
    }

    async fn seed(&self) {
        let client = self.store.session().await.expect("the seed connection");
        client.batch_execute(SEED).await.expect("the seeded rows");
    }

    fn sources(&self) -> TenantSources<'_> {
        let cache: &dyn TagBudgetCache = &self.cache;
        TenantSources {
            database: true,
            organizations: Some(&self.lookup),
            cache: Some(cache),
            tags: Some(self.tags.as_ref()),
            groups: Some(self.groups.as_ref()),
        }
    }

    async fn project(&self, project_id: &str) -> CachedProject {
        let text = self
            .database
            .find_project(project_id)
            .await
            .expect("the project read")
            .expect("the seeded project row");
        serde_json::from_str(&text).expect("the project row decodes")
    }

    async fn close(self) {
        self.pool.close();
        self.database.tasks.close();
        self.database.tasks.wait().await;
        self.fixture
            .close_async()
            .await
            .expect("the fixture teardown");
    }
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["auth-tenant-budgets-postgres-tests"])
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
    .max_size(4)
    .build()
    .expect("the database pool");
    (fixture, pool)
}

fn org_store(store: PostgresStore) -> (Arc<DatabaseOrgStore>, OrgLookup) {
    let database = Arc::new(DatabaseOrgStore {
        reader: store.clone(),
        writer: store,
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    });
    let store_handle: Arc<dyn OrgStore> = Arc::<DatabaseOrgStore>::clone(&database);
    let cache_handle: Arc<dyn OrgCache> = Arc::new(OrgWorld::default());
    let lookup = OrgLookup {
        store: Some(store_handle),
        cache: cache_handle,
        in_memory_ttl: 60.0_f64,
    };
    (database, lookup)
}

async fn grant_rows(
    pool: &Pool,
) -> (
    Box<dyn BudgetRows<TagTable>>,
    Box<dyn BudgetRows<ModelAccessGroupBudgetRow>>,
) {
    let client = Arc::new(pool.get().await.expect("the database connection"));
    let tags = open_grant_rows(
        Arc::clone(&client),
        TagBudgetTable,
        AuthReadAdapter::Tag,
        context(),
    );
    let groups = open_grant_rows(
        client,
        ModelAccessGroupBudgetTable,
        AuthReadAdapter::ModelAccessGroupBudget,
        context(),
    );
    (Box::new(tags), Box::new(groups))
}

fn context() -> ReadContext<SystemClock> {
    ReadContext {
        default_deadline_seconds: 5.0_f64,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }
}

fn assert_denial(
    error: TenantBudgetError,
    entity_type: &str,
    entity_id: &str,
    current_cost: &str,
    max_budget: &str,
) {
    let TenantBudgetError::Denied(denial) = error else {
        panic!("the tenant check must deny");
    };
    assert_eq!(denial.entity_type, entity_type);
    let found = denial
        .entity_id
        .expect("a tenant denial names its entity")
        .expose_secret()
        .to_owned();
    assert_eq!(found, entity_id);
    assert_eq!(denial.current_cost, decimal(current_cost));
    assert_eq!(denial.max_budget, decimal(max_budget));
}

#[tokio::test]
async fn project_denial_at_the_stored_ceiling() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let project = admitted.project("p-spent").await;
    let reader = ScriptedSpend::new(vec![number(10.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk", "org_id": "o-spent"}));
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: Some(&project),
        tags: &[],
        matched_model_access_groups: &[],
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("the stored project ceiling denies at equality");
    assert_denial(error, "project", "p-spent", "10", "10");
    let reads = reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:project:p-spent");
    assert_eq!(reads[0].fallback_spend, decimal("0"));
    assert_eq!(reads[0].max_budget, Some(decimal("10")));
    assert!(!reads[0].fallback_authoritative);
    assert_eq!(recorder.take()[0].kind, TenantAlertKind::ProjectBudget);
    admitted.close().await;
}

#[tokio::test]
async fn organization_denial_at_the_stored_ceiling() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![number(20.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk", "org_id": "o-spent"}));
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &[],
        matched_model_access_groups: &[],
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("the stored organization ceiling denies at equality");
    assert_denial(error, "organization", "o-spent", "20", "20");
    let reads = reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:org:o-spent");
    assert_eq!(reads[0].fallback_spend, decimal("2"));
    assert_eq!(reads[0].max_budget, Some(decimal("20")));
    assert!(!reads[0].fallback_authoritative);
    assert_eq!(recorder.take()[0].kind, TenantAlertKind::OrganizationBudget);
    admitted.close().await;
}

#[tokio::test]
async fn team_organization_denial_reaches_the_stored_ceiling() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![number(20.5)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: None,
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk"}));
    let row = team(Some("o-spent"));
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: Some(&row),
        project: None,
        tags: &[],
        matched_model_access_groups: &[],
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("a team's organization id reaches the same stored ceiling");
    assert_denial(error, "organization", "o-spent", "20.5", "20");
    assert_eq!(reader.reads()[0].fallback_spend, decimal("2"));
    admitted.close().await;
}

#[tokio::test]
async fn tag_denial_a_cent_over_the_stored_ceiling() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![number(5.01)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk"}));
    let names = ["tag-spent".to_owned(), "tag-roomy".to_owned()];
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &names,
        matched_model_access_groups: &[],
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("the stored tag ceiling denies a cent over it");
    assert_denial(error, "tag", "tag-spent", "5.01", "5");
    let reads = reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:tag:tag-spent");
    assert_eq!(reads[0].fallback_spend, decimal("1"));
    assert_eq!(reads[0].max_budget, Some(decimal("5")));
    assert!(reads[0].fallback_authoritative);
    assert!(recorder.take().is_empty());
    admitted.close().await;
}

#[tokio::test]
async fn tag_equality_allows_the_stored_ceiling() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![number(100.0)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: None,
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk"}));
    let roomy = ["tag-roomy".to_owned()];
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &roomy,
        matched_model_access_groups: &[],
    };
    admit(&mut request, &sources, &admission)
        .await
        .expect("a tag spend equal to the stored ceiling stays allowed");
    assert_eq!(reader.reads().len(), 1);
    admitted.close().await;
}

#[tokio::test]
async fn unpriced_tag_skips_the_counter() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: None,
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk"}));
    let unpriced = ["tag-unpriced".to_owned()];
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &unpriced,
        matched_model_access_groups: &[],
    };
    admit(&mut request, &sources, &admission)
        .await
        .expect("a stored budget table without a ceiling skips the tag counter");
    assert!(reader.reads().is_empty());
    admitted.close().await;
}

#[tokio::test]
async fn group_denial_at_the_stored_ceiling() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![number(4.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk"}));
    let matched = ["group-spent".to_owned()];
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &[],
        matched_model_access_groups: &matched,
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("the stored access-group ceiling denies at equality");
    assert_denial(error, "model_access_group", "group-spent", "4", "4");
    let reads = reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:model_access_group:group-spent");
    assert_eq!(reads[0].fallback_spend, decimal("1"));
    assert_eq!(reads[0].max_budget, Some(decimal("4")));
    assert!(reads[0].fallback_authoritative);
    assert!(recorder.take().is_empty());
    admitted.close().await;
}

#[tokio::test]
async fn group_spend_below_the_stored_ceiling() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![number(99.99)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: None,
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk"}));
    let matched = ["group-roomy".to_owned()];
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &[],
        matched_model_access_groups: &matched,
    };
    admit(&mut request, &sources, &admission)
        .await
        .expect("a group spend below its stored ceiling passes");
    assert_eq!(reader.reads()[0].max_budget, Some(decimal("100")));
    admitted.close().await;
}

#[tokio::test]
async fn missing_organization_row_skips() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: None,
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk", "org_id": "o-absent"}));
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &[],
        matched_model_access_groups: &[],
    };
    admit(&mut request, &sources, &admission)
        .await
        .expect("a missing organization row skips the check");
    assert!(reader.reads().is_empty());
    admitted.close().await;
}

#[tokio::test]
async fn unpriced_organization_skips_the_counter() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: None,
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk", "org_id": "o-unpriced"}));
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &[],
        matched_model_access_groups: &[],
    };
    admit(&mut request, &sources, &admission)
        .await
        .expect("a stored organization budget without a ceiling skips the counter");
    assert!(reader.reads().is_empty());
    admitted.close().await;
}

#[tokio::test]
async fn unpriced_project_skips_the_counter() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let project = admitted.project("p-plain").await;
    let reader = ScriptedSpend::new(vec![number(1000.0)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: None,
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk"}));
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: Some(&project),
        tags: &[],
        matched_model_access_groups: &[],
    };
    admit(&mut request, &sources, &admission)
        .await
        .expect("a project without a stored budget row skips the check");
    assert!(reader.reads().is_empty());
    admitted.close().await;
}

#[tokio::test]
async fn missing_tag_does_not_hide_the_group_denial() {
    let admitted = Admitted::provision().await;
    admitted.seed().await;
    let reader = ScriptedSpend::new(vec![number(4.0)]);
    let admission = TenantAdmission {
        reader: &reader,
        alerts: None,
    };
    let sources = admitted.sources();
    let mut token = token(json!({"token": "sk", "org_id": "o-absent"}));
    let names = ["tag-absent".to_owned()];
    let matched = ["group-spent".to_owned()];
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &names,
        matched_model_access_groups: &matched,
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("the missing organization and tag rows skip, the group still denies");
    assert_denial(error, "model_access_group", "group-spent", "4", "4");
    let reads = reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:model_access_group:group-spent");
    admitted.close().await;
}

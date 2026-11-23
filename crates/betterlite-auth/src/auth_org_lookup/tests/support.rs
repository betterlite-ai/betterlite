use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::DbError as PrismaFault;
use betterlite_db::database_faults::ErrorType;
use betterlite_db::postgres_store::diagnostics::PoolFailure;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::resource_rows::project::ProjectTable;
use deadpool_postgres::PoolError;
use serde::de::DeserializeOwned;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::auth_cache::AuthCacheError;
use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer, StoreFailure};
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_read::table::AuthReadError;

#[derive(Default)]
pub(crate) struct State {
    pub(crate) events: Vec<String>,
    pub(crate) orgs: BTreeMap<String, OrganizationTable>,
    pub(crate) projects: BTreeMap<String, CachedProject>,
    pub(crate) org_rows: Vec<OrganizationTable>,
    pub(crate) project_rows: BTreeMap<String, ProjectTable>,
    pub(crate) failing: BTreeSet<String>,
}

// the cache and the repository over one recorded state, so a test reads the whole call order
pub(crate) struct World {
    state: Mutex<State>,
    management_ttl: f64,
}

impl World {
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.lock().events.clone()
    }

    pub(crate) fn clear_events(&self) {
        self.lock().events.clear();
    }

    pub(crate) fn script(&self, name: &str) {
        self.lock().failing.insert(name.to_owned());
    }

    pub(crate) fn seed_org(&self, org: OrganizationTable) {
        self.lock().org_rows.push(org);
    }

    pub(crate) fn hold_org(&self, key: &str, org: OrganizationTable) {
        self.lock().orgs.insert(key.to_owned(), org);
    }

    pub(crate) fn hold_last_known(&self, org_id: &str, org: OrganizationTable) {
        let key = format!("org_id:{org_id}:with_budget:last_known");
        self.lock().orgs.insert(key, org);
    }

    pub(crate) fn cached_org(&self, key: &str) -> Option<OrganizationTable> {
        self.lock().orgs.get(key).cloned()
    }

    pub(crate) fn seed_project(&self, project: ProjectTable) {
        self.lock()
            .project_rows
            .insert(project.project_id.clone(), project);
    }

    pub(crate) fn hold_project(&self, key: &str, project: CachedProject) {
        self.lock().projects.insert(key.to_owned(), project);
    }

    pub(crate) fn cached_project(&self, key: &str) -> Option<CachedProject> {
        self.lock().projects.get(key).cloned()
    }

    fn record(&self, event: String) {
        self.lock().events.push(event);
    }

    fn fails(&self, name: &str) -> bool {
        self.lock().failing.contains(name)
    }

    fn store_fault(&self, name: &str) -> Option<StoreFailure> {
        if self.fails(&format!("outage {name}")) {
            return Some(Box::new(AuthReadError::Repository(outage_fault())));
        }
        if self.fails(&format!("permanent {name}")) {
            return Some(Box::new(AuthReadError::Lookup(DbLookupError::Database(
                permanent_fault(),
            ))));
        }
        if self.fails(&format!("fault {name}")) {
            return Some(Box::new(AuthReadError::Repository(DbError::CreateNull)));
        }
        None
    }
}

fn outage_fault() -> DbError<Infallible> {
    DbError::Pool {
        source: PoolFailure {
            driver: PoolError::Closed,
        },
    }
}

// a prisma-shaped engine fault that retrying cannot clear, so the 503 words it as permanent
fn permanent_fault() -> PrismaFault {
    PrismaFault {
        error_type: ErrorType::Prisma,
        message: "the query engine reported a protocol violation".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

fn cache_failure() -> AuthCacheError {
    AuthCacheError::Backend(Box::new(DualCacheError::Memory(BackendError::Unsupported(
        "scripted",
    ))))
}

pub(crate) fn world() -> Arc<World> {
    Arc::new(World {
        state: Mutex::new(State::default()),
        management_ttl: 60.0_f64,
    })
}

pub(crate) fn lookup(world: &Arc<World>) -> OrgLookup {
    let cache: Arc<dyn OrgCache> = Arc::<World>::clone(world);
    let store: Arc<dyn OrgStore> = Arc::<World>::clone(world);
    OrgLookup {
        store: Some(store),
        cache,
        in_memory_ttl: 5.0_f64,
    }
}

pub(crate) fn without_database(world: &Arc<World>) -> OrgLookup {
    let mut unwired = lookup(world);
    unwired.store = None;
    unwired
}

// a row as the table stores it: the with_budget query joins the budget relation in, the others leave it out
pub(crate) fn row_text(org: &OrganizationTable, include_budget: bool) -> String {
    let mut row = org.clone();
    if !include_budget {
        row.litellm_budget_table = None;
    }
    serde_json::to_string(&row).unwrap()
}

fn decode<T: DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn org_row(org_id: &str, alias: Option<&str>, budget_id: &str) -> OrganizationTable {
    let alias = alias.map_or_else(|| "null".to_owned(), |value| format!("\"{value}\""));
    decode(&format!(
        r#"{{"organization_id": "{org_id}", "organization_alias": {alias}, "budget_id": "{budget_id}", "created_by": "admin", "updated_by": "admin", "models": ["gpt-4o"]}}"#
    ))
}

pub(crate) fn with_budget(mut org: OrganizationTable, max_budget: &str) -> OrganizationTable {
    let budget_id = org.budget_id.clone();
    org.litellm_budget_table = Some(decode(&format!(
        r#"{{"budget_id": "{budget_id}", "max_budget": {max_budget}, "allowed_models": ["gpt-4o"]}}"#
    )));
    org
}

pub(crate) fn project_row(project_id: &str, max_budget: Option<&str>) -> ProjectTable {
    let budget = max_budget.map_or_else(
        || "null".to_owned(),
        |value| format!(r#"{{"budget_id": "b1", "max_budget": {value}}}"#),
    );
    decode(&format!(
        r#"{{"project_id": "{project_id}", "project_alias": "proj", "budget_id": "b1", "litellm_budget_table": {budget}}}"#
    ))
}

pub(crate) fn cached_project(project: ProjectTable) -> CachedProject {
    CachedProject {
        project,
        last_refreshed_at: None,
    }
}

pub(crate) fn assert_events(world: &World, expected: &[&str]) {
    let seen = world.events();
    let wanted: Vec<String> = expected.iter().map(|event| (*event).to_owned()).collect();
    assert_eq!(seen, wanted);
}

#[async_trait]
impl OrgCache for World {
    async fn read_org(
        &self,
        key: &str,
        local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        if self.fails(&format!("read {key}")) {
            return Err(cache_failure());
        }
        self.record(format!("read {key} local={local_only}"));
        Ok(self.lock().orgs.get(key).cloned())
    }

    async fn write_org(
        &self,
        key: &str,
        org: &OrganizationTable,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        if self.fails(&format!("write {key}")) {
            return Err(cache_failure());
        }
        self.record(format!("write {key} ttl={ttl}"));
        self.lock().orgs.insert(key.to_owned(), org.clone());
        Ok(())
    }

    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        if self.fails(&format!("read {key}")) {
            return Err(cache_failure());
        }
        self.record(format!("read {key}"));
        Ok(self.lock().projects.get(key).cloned())
    }

    async fn write_project(
        &self,
        key: &str,
        project: &CachedProject,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        if self.fails(&format!("write {key}")) {
            return Err(cache_failure());
        }
        self.record(format!("write {key} ttl={ttl}"));
        self.lock().projects.insert(key.to_owned(), project.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        self.management_ttl
    }
}

#[async_trait]
impl OrgStore for World {
    async fn find_org(&self, org_id: &str, include_budget: bool) -> StoreAnswer<Option<String>> {
        if let Some(fault) = self.store_fault("org") {
            return Err(fault);
        }
        self.record(format!("org {org_id} budget={include_budget}"));
        let found = self
            .lock()
            .org_rows
            .iter()
            .find(|row| row.organization_id.as_deref() == Some(org_id))
            .cloned();
        Ok(found.map(|org| row_text(&org, include_budget)))
    }

    async fn find_by_alias(&self, org_alias: &str) -> StoreAnswer<Vec<String>> {
        if let Some(fault) = self.store_fault("alias") {
            return Err(fault);
        }
        self.record(format!("alias {org_alias}"));
        let rows: Vec<OrganizationTable> = self
            .lock()
            .org_rows
            .iter()
            .filter(|row| row.organization_alias.as_deref() == Some(org_alias))
            .cloned()
            .collect();
        Ok(rows.iter().map(|org| row_text(org, false)).collect())
    }

    async fn find_project(&self, project_id: &str) -> StoreAnswer<Option<String>> {
        if let Some(fault) = self.store_fault("project") {
            return Err(fault);
        }
        self.record(format!("project {project_id}"));
        let found = self.lock().project_rows.get(project_id).cloned();
        Ok(found.map(|project| {
            serde_json::to_string(&CachedProject {
                project,
                last_refreshed_at: None,
            })
            .unwrap()
        }))
    }
}

// the levels this module logged, in order
#[derive(Clone, Default)]
pub(crate) struct Levels {
    seen: Arc<Mutex<Vec<Level>>>,
}

impl Levels {
    pub(crate) fn install(&self) -> DefaultGuard {
        set_default(tracing_subscriber::registry().with(self.clone()))
    }

    pub(crate) fn seen(&self) -> Vec<Level> {
        self.seen.lock().unwrap().clone()
    }
}

impl<S: Subscriber> Layer<S> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event
            .metadata()
            .target()
            .starts_with("betterlite_auth::auth_org_lookup")
        {
            self.seen.lock().unwrap().push(*event.metadata().level());
        }
    }
}

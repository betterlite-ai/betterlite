use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer, StoreFailure};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::resource_rows::project::ProjectTable;

use super::input::{Function, Input, Step};

// the management-object ttl both sides write: the cache fixture and LiteLLM's UserApiKeyCache share it
const MANAGEMENT_TTL_SECONDS: f64 = 60.0;

#[derive(Default)]
pub(crate) struct Board {
    pub(crate) events: Vec<String>,
    pub(crate) logs: Vec<String>,
    fail: Vec<String>,
}

// the org cache, the project cache and the two tables, recording every read and write in order
pub(crate) struct Script {
    board: Mutex<Board>,
    values: Mutex<BTreeMap<String, OrganizationTable>>,
    projects: Mutex<BTreeMap<String, CachedProject>>,
    org_rows: Vec<OrganizationTable>,
    project_row: Option<ProjectTable>,
}

impl Script {
    pub(crate) fn new(input: &Input) -> Self {
        Self {
            board: Mutex::new(Board::default()),
            values: Mutex::new(seeded_orgs(input)),
            projects: Mutex::new(seeded_projects(input)),
            org_rows: input
                .org_row
                .iter()
                .chain(input.alias_rows.iter())
                .cloned()
                .collect(),
            project_row: input.project_row.clone(),
        }
    }

    pub(crate) fn begin(&self, step: &Step) {
        let mut board = self.board();
        board.events.clear();
        board.logs.clear();
        board.fail.clone_from(&step.fail);
    }

    pub(crate) fn snapshot(&self) -> (Vec<String>, Vec<String>) {
        let board = self.board();
        (board.events.clone(), board.logs.clone())
    }

    pub(crate) fn record_log(&self, level: String) {
        self.board().logs.push(level);
    }

    fn board(&self) -> MutexGuard<'_, Board> {
        self.board.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn values(&self) -> MutexGuard<'_, BTreeMap<String, OrganizationTable>> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn projects(&self) -> MutexGuard<'_, BTreeMap<String, CachedProject>> {
        self.projects.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, event: String) {
        self.board().events.push(event);
    }

    fn fails(&self, name: &str) -> bool {
        self.board().fail.iter().any(|script| script == name)
    }

    fn store_failure(&self) -> Option<StoreFailure> {
        if self.fails("outage") {
            return Some(Box::new(AuthReadError::Repository(DbError::NoDbConnected)));
        }
        if self.fails("fault") {
            return Some(Box::new(AuthReadError::Repository(DbError::CreateNull)));
        }
        None
    }
}

fn seeded_orgs(input: &Input) -> BTreeMap<String, OrganizationTable> {
    let mut values = BTreeMap::new();
    let target = &input.target;
    let cached_org = input.cached_org.as_ref();
    let cached_last_known = input.cached_last_known.as_ref();
    match input.function {
        Function::OrgObject => {
            if let Some(org) = cached_org {
                let _previous = values.insert(org_key(target, input.include_budget), org.clone());
            }
        }
        Function::OrgAlias => {
            if let Some(org) = cached_org {
                let _previous = values.insert(format!("org_alias:{target}"), org.clone());
            }
        }
        Function::OrgForRequest => {
            if let Some(org) = cached_org {
                let _previous = values.insert(org_key(target, true), org.clone());
            }
            if let Some(stale) = cached_last_known {
                let _previous = values.insert(last_known_key(target), stale.clone());
            }
        }
        Function::ProjectObject => {}
    }
    values
}

fn seeded_projects(input: &Input) -> BTreeMap<String, CachedProject> {
    let mut projects = BTreeMap::new();
    match input.function {
        Function::ProjectObject => {
            if let Some(project) = &input.cached_project {
                let _previous = projects.insert(project_key(&input.target), project.clone());
            }
        }
        Function::OrgObject | Function::OrgAlias | Function::OrgForRequest => {}
    }
    projects
}

pub(crate) fn org_key(org_id: &str, include_budget: bool) -> String {
    if include_budget {
        return format!("org_id:{org_id}:with_budget");
    }
    format!("org_id:{org_id}")
}

pub(crate) fn last_known_key(org_id: &str) -> String {
    format!("org_id:{org_id}:with_budget:last_known")
}

pub(crate) fn project_key(project_id: &str) -> String {
    format!("project_id:{project_id}")
}

fn cache_failure() -> AuthCacheError {
    AuthCacheError::Payload(Box::new(PayloadError::Json(serde_json::Error::io(
        IoError::other("scripted cache write failure"),
    ))))
}

// what Prisma answers for an organization row: the include decides whether the budget relation rides along
fn row_text(org: &OrganizationTable, include_budget: bool) -> String {
    let mut row = org.clone();
    if !include_budget {
        row.litellm_budget_table = None;
    }
    serde_json::to_string(&row).unwrap_or_default()
}

fn project_text(project: ProjectTable) -> String {
    let cached = CachedProject {
        project,
        last_refreshed_at: None,
    };
    serde_json::to_string(&cached).unwrap_or_default()
}

#[async_trait]
impl OrgCache for Script {
    async fn read_org(
        &self,
        key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        self.record(format!("read {key}"));
        let cached = self.values().get(key).cloned();
        Ok(cached)
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
        let _previous = self.values().insert(key.to_owned(), org.clone());
        Ok(())
    }

    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        self.record(format!("read {key}"));
        let cached = self.projects().get(key).cloned();
        Ok(cached)
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
        let _previous = self.projects().insert(key.to_owned(), project.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        MANAGEMENT_TTL_SECONDS
    }
}

#[async_trait]
impl OrgStore for Script {
    async fn find_org(&self, org_id: &str, include_budget: bool) -> StoreAnswer<Option<String>> {
        if let Some(fault) = self.store_failure() {
            return Err(fault);
        }
        self.record(format!(
            "find_unique organization {org_id} include={include_budget}"
        ));
        let found = self
            .org_rows
            .iter()
            .find(|row| row.organization_id.as_deref() == Some(org_id))
            .cloned();
        Ok(found.map(|org| row_text(&org, include_budget)))
    }

    async fn find_by_alias(&self, org_alias: &str) -> StoreAnswer<Vec<String>> {
        if let Some(fault) = self.store_failure() {
            return Err(fault);
        }
        self.record(format!("find_many organization {org_alias}"));
        Ok(self
            .org_rows
            .iter()
            .filter(|row| row.organization_alias.as_deref() == Some(org_alias))
            .map(|row| row_text(row, false))
            .collect())
    }

    async fn find_project(&self, project_id: &str) -> StoreAnswer<Option<String>> {
        if let Some(fault) = self.store_failure() {
            return Err(fault);
        }
        self.record(format!("find_unique project {project_id} include=true"));
        let found = self
            .project_row
            .as_ref()
            .filter(|row| row.project_id == project_id)
            .cloned();
        Ok(found.map(project_text))
    }
}

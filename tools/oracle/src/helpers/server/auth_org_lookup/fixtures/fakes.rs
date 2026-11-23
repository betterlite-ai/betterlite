use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::sync::{Mutex, PoisonError};

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

// the cache trait's fallible write answer, so each impl names it once
type CacheWrite = Result<(), AuthCacheError>;

// the two management objects the cache writes, so one helper carries the write order and failure
enum Entry<'a> {
    Org(&'a OrganizationTable),
    Project(&'a CachedProject),
}

#[derive(Default)]
pub(crate) struct Board {
    pub(crate) events: Vec<String>,
    pub(crate) logs: Vec<String>,
    fail: Vec<String>,
}

// the org cache, the project cache and the two tables, recording every read and write in order
pub(crate) struct Script {
    pub(crate) board: Mutex<Board>,
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
        let mut board = self.board.lock().unwrap_or_else(PoisonError::into_inner);
        board.events.clear();
        board.logs.clear();
        board.fail.clone_from(&step.fail);
    }

    fn store_failure(&self) -> Option<StoreFailure> {
        let board = self.board.lock().unwrap_or_else(PoisonError::into_inner);
        if board.fail.iter().any(|script| script == "outage") {
            return Some(Box::new(AuthReadError::Repository(DbError::NoDbConnected)));
        }
        if board.fail.iter().any(|script| script == "fault") {
            return Some(Box::new(AuthReadError::Repository(DbError::CreateNull)));
        }
        None
    }

    fn begin_read(&self, event: String) -> Result<(), StoreFailure> {
        match self.store_failure() {
            Some(fault) => Err(fault),
            None => {
                self.board
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .events
                    .push(event);
                Ok(())
            }
        }
    }

    // both cache reads record the key on the board and then clone the stored object
    fn read_map<V: Clone>(&self, map: &Mutex<BTreeMap<String, V>>, key: &str) -> Option<V> {
        self.board
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .events
            .push(format!("read {key}"));
        let stored = map.lock().unwrap_or_else(PoisonError::into_inner);
        let found = stored.get(key);
        found.cloned()
    }

    fn accept_write(&self, key: &str, entry: Entry<'_>, ttl: f64) -> CacheWrite {
        {
            let mut board = self.board.lock().unwrap_or_else(PoisonError::into_inner);
            if board
                .fail
                .iter()
                .any(|script| script == &format!("write {key}"))
            {
                return Err(cache_failure());
            }
            board.events.push(format!("write {key} ttl={ttl}"));
        }
        match entry {
            Entry::Org(org) => {
                let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
                let _previous = values.insert(key.to_owned(), org.clone());
            }
            Entry::Project(project) => {
                let projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
                let _previous = projects.insert(key.to_owned(), project.clone());
            }
        }
        Ok(())
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
                let _previous = values.insert(
                    format!("org_id:{target}:with_budget:last_known"),
                    stale.clone(),
                );
            }
        }
        Function::ProjectObject => {}
    }
    values
}

fn seeded_projects(input: &Input) -> BTreeMap<String, CachedProject> {
    let mut projects = BTreeMap::new();
    let target = &input.target;
    match input.function {
        Function::ProjectObject => {
            if let Some(project) = &input.cached_project {
                let _previous = projects.insert(format!("project_id:{target}"), project.clone());
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
        Ok(self.read_map(&self.values, key))
    }

    async fn write_org(&self, key: &str, org: &OrganizationTable, ttl: f64) -> CacheWrite {
        self.accept_write(key, Entry::Org(org), ttl)
    }

    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        Ok(self.read_map(&self.projects, key))
    }

    async fn write_project(&self, key: &str, project: &CachedProject, ttl: f64) -> CacheWrite {
        self.accept_write(key, Entry::Project(project), ttl)
    }

    fn management_ttl(&self) -> f64 {
        MANAGEMENT_TTL_SECONDS
    }
}

#[async_trait]
impl OrgStore for Script {
    async fn find_org(&self, org_id: &str, include_budget: bool) -> StoreAnswer<Option<String>> {
        self.begin_read(format!(
            "find_unique organization {org_id} include={include_budget}"
        ))?;
        let found = self
            .org_rows
            .iter()
            .find(|row| row.organization_id.as_deref() == Some(org_id))
            .cloned();
        Ok(found.map(|org| row_text(&org, include_budget)))
    }

    async fn find_by_alias(&self, org_alias: &str) -> StoreAnswer<Vec<String>> {
        self.begin_read(format!("find_many organization {org_alias}"))?;
        Ok(self
            .org_rows
            .iter()
            .filter(|row| row.organization_alias.as_deref() == Some(org_alias))
            .map(|row| row_text(row, false))
            .collect())
    }

    async fn find_project(&self, project_id: &str) -> StoreAnswer<Option<String>> {
        self.begin_read(format!("find_unique project {project_id} include=true"))?;
        let row = self
            .project_row
            .as_ref()
            .filter(|row| row.project_id == project_id);
        Ok(row.map(|project| project_text(project.clone())))
    }
}

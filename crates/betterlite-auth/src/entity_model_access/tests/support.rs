use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{
    JsonData, LiteLLMBudgetTable, LiteLLMObjectPermissionTable,
};
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::auth_cache::AuthCacheError;
use crate::auth_end_users::defaults::MemberBudgetError;
use crate::auth_key_budgets::spend::{SpendReadError, SpendReader};
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::pattern::UnresolvedProviderInference;
use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer as OrgAnswer};
use crate::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use crate::auth_prefetch::permission::store::ObjectPermissionStore;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use crate::auth_team_lookup::ports::{StoreAnswer, TeamCache, TeamCacheError, TeamStore};
use crate::auth_user_lookup::cadence::{Cadence, WallClock};
use crate::budget_primitives::window_start::DurationParser;
use crate::current_spend::SpendQuery;
use crate::membership_load::error::MembershipLoadError;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use crate::team_budget_admission::TeamAdmission;
use crate::team_budget_admission::sources::{DefaultBudgetSource, MembershipSource};

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

// every entity read this module makes, served from the seeds the test writes
#[derive(Default)]
pub(crate) struct Script {
    pub(crate) teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    pub(crate) groups: BTreeMap<String, AccessGroupTable>,
    pub(crate) projects: Mutex<BTreeMap<String, CachedProject>>,
    pub(crate) memberships: BTreeMap<String, LiteLLMTeamMembership>,
    pub(crate) events: Mutex<Vec<String>>,
}

impl Script {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub(crate) fn with_team(mut self, team: LiteLLMTeamTable) -> Self {
        let id = team.fields.base.team_id.clone();
        self.teams.insert(
            id,
            LiteLLMTeamTableCachedObj {
                team,
                last_refreshed_at: None,
            },
        );
        self
    }

    #[must_use]
    pub(crate) fn with_group(mut self, group: AccessGroupTable) -> Self {
        self.groups.insert(group.access_group_id.clone(), group);
        self
    }

    #[must_use]
    pub(crate) fn with_project(mut self, project: ProjectTable) -> Self {
        let entry = CachedProject {
            project,
            last_refreshed_at: None,
        };
        self.projects
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(entry.project.project_id.clone(), entry);
        self
    }

    #[must_use]
    pub(crate) fn with_membership(mut self, user_id: &str, team_id: &str, text: &str) -> Self {
        let row = membership_row(text);
        self.memberships.insert(format!("{user_id}:{team_id}"), row);
        self
    }

    pub(crate) fn reads(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, event: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

fn cache_key<'key>(key: &'key str, prefix: &str) -> &'key str {
    key.strip_prefix(prefix).unwrap_or(key)
}

#[async_trait]
impl TeamCache for Script {
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        self.record(format!("read {key}"));
        Ok(self.teams.get(cache_key(key, "team_id:")).cloned())
    }

    async fn write_team(
        &self,
        _key: &str,
        _team: &LiteLLMTeamTableCachedObj,
        _ttl: f64,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn cache_team(
        &self,
        _team_id: &str,
        _team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn read_access_group(
        &self,
        key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        self.record(format!("read {key}"));
        Ok(self.groups.get(cache_key(key, "access_group_id:")).cloned())
    }

    async fn cache_access_group(
        &self,
        _access_group_id: &str,
        _table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }
}

#[async_trait]
impl TeamStore for Script {
    async fn find_team(&self, team_id: &str, _use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        self.record(format!("find team {team_id}"));
        Ok(None)
    }

    async fn find_by_alias(&self, team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        self.record(format!("find team alias {team_alias}"));
        Ok(Vec::new())
    }

    async fn find_access_group(
        &self,
        access_group_id: &str,
        _use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        self.record(format!("find group {access_group_id}"));
        Ok(None)
    }
}

#[async_trait]
impl ObjectPermissionCache for Script {
    async fn read(&self, key: &str) -> PermissionAnswer {
        self.record(format!("read {key}"));
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        _ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl ObjectPermissionStore for Script {
    async fn find_permission(&self, id: &str) -> Result<Option<JsonData>, PermissionError> {
        self.record(format!("find permission {id}"));
        Ok(None)
    }
}

#[async_trait]
impl OrgCache for Script {
    async fn read_org(
        &self,
        key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        self.record(format!("read {key}"));
        Ok(None)
    }

    async fn write_org(
        &self,
        _key: &str,
        _org: &OrganizationTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        self.record(format!("read {key}"));
        let projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(projects.get(cache_key(key, "project_id:")).cloned())
    }

    async fn write_project(
        &self,
        key: &str,
        project: &CachedProject,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.projects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(cache_key(key, "project_id:").to_owned(), project.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl OrgStore for Script {
    async fn find_org(&self, org_id: &str, _include_budget: bool) -> OrgAnswer<Option<String>> {
        self.record(format!("find org {org_id}"));
        Ok(None)
    }

    async fn find_by_alias(&self, org_alias: &str) -> OrgAnswer<Vec<String>> {
        self.record(format!("find org alias {org_alias}"));
        Ok(Vec::new())
    }

    async fn find_project(&self, project_id: &str) -> OrgAnswer<Option<String>> {
        self.record(format!("find project {project_id}"));
        Ok(None)
    }
}

#[async_trait]
impl MembershipSource for Script {
    async fn team_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.record(format!("read membership {user_id}:{team_id}"));
        Ok(self
            .memberships
            .get(&format!("{user_id}:{team_id}"))
            .cloned())
    }
}

#[async_trait]
impl SpendReader for Script {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}

#[async_trait]
impl DefaultBudgetSource for Script {
    async fn team_member_default(
        &self,
        _budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        Ok(None)
    }
}

impl DurationParser for Script {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

#[must_use]
pub(crate) fn team_lookup(script: &Arc<Script>) -> TeamLookup {
    TeamLookup {
        cache: Arc::clone(script),
        permission_cache: Arc::clone(script),
        database: Some(TeamDatabase {
            store: Arc::clone(script),
            permission_reader: Arc::clone(script),
            permission_writer: Arc::clone(script),
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(1024, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

#[must_use]
pub(crate) fn project_lookup(script: &Arc<Script>) -> OrgLookup {
    OrgLookup {
        store: Some(Arc::clone(script)),
        cache: Arc::clone(script),
        in_memory_ttl: 60.0,
    }
}

#[must_use]
pub(crate) fn admission<'deps>(
    script: &'deps Script,
    models: &'deps ModelAccessContext<'deps>,
    now: DateTime<Utc>,
) -> TeamAdmission<'deps> {
    TeamAdmission {
        reader: script,
        memberships: script,
        defaults: script,
        member_alerts: None,
        team_alerts: None,
        durations: script,
        models,
        now,
    }
}

pub(crate) struct Setup {
    pub(crate) aliases: BTreeMap<String, String>,
    pub(crate) max_depth: i64,
}

impl Setup {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            aliases: BTreeMap::new(),
            max_depth: 100,
        }
    }

    #[must_use]
    pub(crate) fn context(&self) -> ModelAccessContext<'_> {
        ModelAccessContext {
            view: None,
            global_model_aliases: &self.aliases,
            compaction: None,
            provider: &UNRESOLVED_PROVIDER,
            max_recurse_depth: self.max_depth,
        }
    }
}

#[must_use]
pub(crate) fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

#[must_use]
pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

#[must_use]
pub(crate) fn token(text: &str) -> UserAPIKeyAuth {
    let JsonData::Object(fields) = json(text) else {
        panic!("the token fixture must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the token fixture constructs")
}

#[must_use]
pub(crate) fn team_row(text: &str) -> LiteLLMTeamTable {
    LiteLLMTeamTable::try_from(json(text)).expect("the team fixture validates")
}

#[must_use]
pub(crate) fn group_row(text: &str) -> AccessGroupTable {
    serde_json::from_str(text).expect("the group fixture decodes")
}

#[must_use]
pub(crate) fn org_row(text: &str) -> OrganizationTable {
    serde_json::from_str(text).expect("the organization fixture decodes")
}

#[must_use]
pub(crate) fn project_row(text: &str) -> ProjectTable {
    serde_json::from_str(text).expect("the project fixture decodes")
}

#[must_use]
pub(crate) fn membership_row(text: &str) -> LiteLLMTeamMembership {
    serde_json::from_str(text).expect("the membership fixture decodes")
}

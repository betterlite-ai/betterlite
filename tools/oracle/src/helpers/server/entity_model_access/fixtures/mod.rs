use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_end_users::defaults::MemberBudgetError;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_auth::team_budget_admission::TeamAdmission;
use betterlite_auth::team_budget_admission::sources::{DefaultBudgetSource, MembershipSource};
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::Step;

mod tables;

// every entity the case seeds into the proxy cache, keyed exactly the way LiteLLM keys it
#[derive(Default)]
pub(super) struct Fixture {
    teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    groups: BTreeMap<String, AccessGroupTable>,
    projects: Mutex<BTreeMap<String, CachedProject>>,
    memberships: BTreeMap<String, LiteLLMTeamMembership>,
}

impl Fixture {
    pub(super) fn build(step: &Step) -> Result<Self, serde_json::Error> {
        let mut fixture = Self::default();
        for (key, value) in &step.cache {
            if let Some(id) = key.strip_prefix("team_id:") {
                let team: LiteLLMTeamTable = serde_json::from_value(value.clone())?;
                fixture.teams.insert(
                    id.to_owned(),
                    LiteLLMTeamTableCachedObj {
                        team,
                        last_refreshed_at: None,
                    },
                );
            } else if let Some(id) = key.strip_prefix("access_group_id:") {
                fixture
                    .groups
                    .insert(id.to_owned(), serde_json::from_value(value.clone())?);
            } else if let Some(id) = key.strip_prefix("project_id:") {
                let project: ProjectTable = serde_json::from_value(value.clone())?;
                fixture
                    .projects
                    .get_mut()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(
                        id.to_owned(),
                        CachedProject {
                            project,
                            last_refreshed_at: None,
                        },
                    );
            } else {
                let pair = key.strip_prefix("team_membership:").unwrap_or(key);
                fixture
                    .memberships
                    .insert(pair.to_owned(), serde_json::from_value(value.clone())?);
            }
        }
        Ok(fixture)
    }
}

#[async_trait]
impl MembershipSource for Fixture {
    async fn team_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        Ok(self
            .memberships
            .get(&format!("{user_id}:{team_id}"))
            .cloned())
    }
}

#[async_trait]
impl SpendReader for Fixture {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}

#[async_trait]
impl DefaultBudgetSource for Fixture {
    async fn team_member_default(
        &self,
        _budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        Ok(None)
    }
}

impl DurationParser for Fixture {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

pub(super) fn team_lookup(fixture: &Arc<Fixture>) -> TeamLookup {
    TeamLookup {
        cache: Arc::<Fixture>::clone(fixture),
        permission_cache: Arc::<Fixture>::clone(fixture),
        database: Some(TeamDatabase {
            store: Arc::<Fixture>::clone(fixture),
            permission_reader: Arc::<Fixture>::clone(fixture),
            permission_writer: Arc::<Fixture>::clone(fixture),
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(1024, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

pub(super) fn project_lookup(fixture: &Arc<Fixture>) -> OrgLookup {
    OrgLookup {
        store: Some(Arc::<Fixture>::clone(fixture)),
        cache: Arc::<Fixture>::clone(fixture),
        in_memory_ttl: 60.0,
    }
}

pub(super) const fn admission<'deps>(
    fixture: &'deps Fixture,
    models: &'deps ModelAccessContext<'deps>,
) -> TeamAdmission<'deps> {
    TeamAdmission {
        reader: fixture,
        memberships: fixture,
        defaults: fixture,
        member_alerts: None,
        team_alerts: None,
        durations: fixture,
        models,
        now: DateTime::UNIX_EPOCH,
    }
}

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;
static NO_ALIASES: BTreeMap<String, String> = BTreeMap::new();
// litellm.constants' DEFAULT_MAX_RECURSE_DEPTH default
const MAX_RECURSE_DEPTH: i64 = 100;

pub(super) const fn context() -> ModelAccessContext<'static> {
    ModelAccessContext {
        view: None,
        global_model_aliases: &NO_ALIASES,
        compaction: None,
        provider: &UNRESOLVED_PROVIDER,
        max_recurse_depth: MAX_RECURSE_DEPTH,
    }
}

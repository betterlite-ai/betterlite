use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use betterlite_auth::agent_access_groups::ceiling::AgentAccessGroupCeiling;
use betterlite_auth::agent_access_groups::error::AccessGroupsError;
use betterlite_auth::agent_caller::error::AgentCallerError;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::auth_end_users::defaults::MemberBudgetError;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_model_grants::access_source::AgentAccessSource;
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::team_budget_admission::TeamAdmission;
use betterlite_auth::team_budget_admission::sources::{DefaultBudgetSource, MembershipSource};
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::Step;
use super::effects::Effects;

// the case's scripted agent collaborators; every call is recorded so a case sees which check ran
pub(super) struct Script<'step> {
    pub(super) step: &'step Step,
    pub(super) effects: Effects,
}

#[async_trait]
impl AgentAccessSource for Script<'_> {
    fn managed_policy(&self, _auth: Option<&UserAPIKeyAuth>) -> Option<AgentResponse> {
        self.effects.record(String::from("managed_policy"));
        let row = self.step.managed.clone()?;
        serde_json::from_value(row).ok()
    }

    async fn managed_ceilings(
        &self,
        policy: &AgentResponse,
    ) -> Result<Vec<AgentAccessGroupCeiling>, AccessGroupsError> {
        self.effects
            .record(format!("managed_ceilings:{}", policy.agent_id));
        Ok(self
            .step
            .managed_ceilings
            .iter()
            .map(|models| ceiling(models.as_slice()))
            .collect())
    }

    async fn unmanaged_ceiling(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentAccessGroupCeiling>, AccessGroupsError> {
        self.effects.record(format!("unmanaged_ceiling:{agent_id}"));
        Ok(self.step.unmanaged.as_deref().map(ceiling))
    }

    async fn caller_auth(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<UserAPIKeyAuth>, AgentCallerError> {
        Ok(None)
    }

    async fn caller_team(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, AgentCallerError> {
        Ok(None)
    }

    async fn caller_user(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMUserTable>, AgentCallerError> {
        Ok(None)
    }
}

fn ceiling(models: &[String]) -> AgentAccessGroupCeiling {
    AgentAccessGroupCeiling {
        access_group_ids: vec![String::from("group-1")],
        models: models.iter().cloned().collect(),
        mcp_server_ids: BTreeSet::new(),
        agent_ids: BTreeSet::new(),
    }
}

// the caller stage's admission with no spend, membership or default budget rows to read
pub(super) struct NoAdmission;

#[async_trait]
impl MembershipSource for NoAdmission {
    async fn team_membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        Ok(None)
    }
}

#[async_trait]
impl SpendReader for NoAdmission {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}

#[async_trait]
impl DefaultBudgetSource for NoAdmission {
    async fn team_member_default(
        &self,
        _budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        Ok(None)
    }
}

impl DurationParser for NoAdmission {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

static NO_ADMISSION: NoAdmission = NoAdmission;
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

pub(super) const fn admission<'deps>(
    models: &'deps ModelAccessContext<'deps>,
) -> TeamAdmission<'deps> {
    TeamAdmission {
        reader: &NO_ADMISSION,
        memberships: &NO_ADMISSION,
        defaults: &NO_ADMISSION,
        member_alerts: None,
        team_alerts: None,
        durations: &NO_ADMISSION,
        models,
        now: DateTime::UNIX_EPOCH,
    }
}

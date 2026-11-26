use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use betterlite_auth::auth_end_users::defaults::MemberBudgetError;
use betterlite_auth::auth_key_budgets::Admission;
use betterlite_auth::auth_key_budgets::KeyBudgetError;
use betterlite_auth::auth_key_budgets::alerts::BudgetAlertSink;
use betterlite_auth::auth_key_budgets::effective_budgets;
use betterlite_auth::auth_key_budgets::key_budget::{KeyBudgets, virtual_key_max_budget_check};
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_key_budgets::spend::SpendReader;
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::{
    TeamLookup, TeamRequest as LookupRequest, TeamSource,
};
use betterlite_auth::auth_team_lookup::team::get_team_object;
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::team_budget_admission::TeamAdmission;
use betterlite_auth::team_budget_admission::TeamRequest;
use betterlite_auth::team_budget_admission::alerts::TeamAlertSink;
use betterlite_auth::team_budget_admission::error::TeamBudgetError;
use betterlite_auth::team_budget_admission::hard::team_max_budget_check;
use betterlite_auth::team_budget_admission::sources::{
    DefaultBudgetSource, Membership, MembershipSource,
};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

mod tests;

// the collaborators the advisory gate reads through: the request path's spend reader, its cache-only team lookup and the alert sinks
pub struct AdvisoryDeps<'deps> {
    pub spend: &'deps dyn SpendReader,
    pub teams: &'deps dyn CachedTeams,
    pub key_alerts: Option<&'deps dyn BudgetAlertSink>,
    pub team_alerts: Option<&'deps dyn TeamAlertSink>,
    pub throttle_pct: Option<f64>,
    pub now: DateTime<Utc>,
}

pub type TeamFuture<'lookup> = Pin<
    Box<dyn Future<Output = Result<LiteLLMTeamTableCachedObj, TeamLookupError>> + Send + 'lookup>,
>;

// auth_checks get_team_object(check_cache_only=True): an absent team is an error the gate swallows, not a database read
pub trait CachedTeams: Send + Sync {
    fn cached_team<'lookup>(&'lookup self, team_id: &'lookup str) -> TeamFuture<'lookup>;
}

impl CachedTeams for TeamLookup {
    fn cached_team<'lookup>(&'lookup self, team_id: &'lookup str) -> TeamFuture<'lookup> {
        let request = LookupRequest {
            team_id,
            source: TeamSource::CacheOnly,
            team_id_upsert: false,
        };
        Box::pin(get_team_object(self, request))
    }
}

// how the key's own check settled: only a denial withholds, and only an unreadable check stops the team check behind it
enum KeyGate {
    Within,
    OverBudget,
    Unreadable,
}

// _key_or_team_is_over_budget: advisory and fail-open, because real traffic on an over-budget key never reaches the success hook
pub async fn key_or_team_over_budget(
    metadata: &IndexMap<String, Json>,
    deps: &AdvisoryDeps<'_>,
) -> bool {
    let Some(mut auth) = caller_auth(metadata) else {
        return false;
    };
    let budgets = match effective_budgets(&auth, deps.now) {
        Ok(budgets) => budgets,
        Err(error) => {
            tracing::debug!("shadow_eval: budget read failed: {error}");
            return false;
        }
    };
    match key_gate(&mut auth, &budgets, deps).await {
        KeyGate::OverBudget => return true,
        KeyGate::Unreadable => return false,
        KeyGate::Within => {}
    }
    // the key's team, read from its serde form because the token view keeps the field private
    let identity: Identity = serde_json::to_value(&auth)
        .and_then(serde_json::from_value)
        .unwrap_or(Identity { team_id: None });
    let Some(team_id) = identity.team_id.filter(|team_id| !team_id.is_empty()) else {
        return false;
    };
    team_is_over_budget(&auth, &team_id, deps).await
}

async fn key_gate(
    auth: &mut UserAPIKeyAuth,
    budgets: &KeyBudgets,
    deps: &AdvisoryDeps<'_>,
) -> KeyGate {
    let admission = Admission {
        reader: deps.spend,
        alerts: deps.key_alerts,
        durations: &NO_SOURCES,
        global_alert_emails: None,
        alert_fraction: Money::Finite(Decimal::ZERO),
        throttle_pct: deps.throttle_pct,
        now: deps.now,
        user_email: None,
    };
    match virtual_key_max_budget_check(auth, budgets, &admission).await {
        Ok(()) => KeyGate::Within,
        Err(KeyBudgetError::Denied(_)) => KeyGate::OverBudget,
        Err(error) => {
            tracing::debug!("shadow_eval: budget read failed: {error}");
            KeyGate::Unreadable
        }
    }
}

// the team's cache-only row: an absent or unreadable team is not an over-budget signal
async fn cached_team_row(
    deps: &AdvisoryDeps<'_>,
    team_id: &str,
) -> Option<LiteLLMTeamTableCachedObj> {
    match deps.teams.cached_team(team_id).await {
        Ok(team) => Some(team),
        Err(error) => {
            tracing::debug!("shadow_eval: budget read failed: {error}");
            None
        }
    }
}

async fn team_is_over_budget(
    auth: &UserAPIKeyAuth,
    team_id: &str,
    deps: &AdvisoryDeps<'_>,
) -> bool {
    let Some(team) = cached_team_row(deps, team_id).await else {
        return false;
    };
    let aliases = BTreeMap::new();
    let models = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &UNRESOLVED_PROVIDER,
        max_recurse_depth: 0_i64,
    };
    let admission = TeamAdmission {
        reader: deps.spend,
        memberships: &NO_SOURCES,
        defaults: &NO_SOURCES,
        member_alerts: None,
        team_alerts: deps.team_alerts,
        durations: &NO_SOURCES,
        models: &models,
        now: deps.now,
    };
    let request = TeamRequest {
        team: Some(&team.team),
        token: Some(auth),
        user_email: None,
        model: &RequestedModels::None,
        membership: Membership::Unloaded,
    };
    match team_max_budget_check(&admission, &request).await {
        Ok(()) => false,
        Err(TeamBudgetError::Denied(_)) => true,
        Err(error) => {
            tracing::debug!("shadow_eval: budget read failed: {error}");
            false
        }
    }
}

// LiteLLM's isinstance(metadata["user_api_key_auth"], UserAPIKeyAuth): anything else, or a dict the key cannot decode, is not the caller
fn caller_auth(metadata: &IndexMap<String, Json>) -> Option<UserAPIKeyAuth> {
    let Json::Object(fields) = metadata.get("user_api_key_auth")? else {
        return None;
    };
    let decoded: BTreeMap<String, JsonData> = serde_json::to_value(fields)
        .and_then(serde_json::from_value)
        .ok()?;
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).ok()
}

#[derive(Deserialize)]
struct Identity {
    #[serde(default)]
    team_id: Option<String>,
}

// the advisory check reads the stored spend alone: no reset window, no membership and no member default reaches it
struct NoSources;

impl DurationParser for NoSources {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

#[async_trait]
impl MembershipSource for NoSources {
    async fn team_membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        Err(MembershipLoadError::NoDatabase)
    }
}

#[async_trait]
impl DefaultBudgetSource for NoSources {
    async fn team_member_default(
        &self,
        _budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        Ok(None)
    }
}

static NO_SOURCES: NoSources = NoSources;

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

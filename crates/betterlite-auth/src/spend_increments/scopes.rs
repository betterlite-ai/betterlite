use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use chrono::{DateTime, Utc};
use futures_util::future::join_all;
use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::auth_model_lists::lists::dedup_first;
use crate::reservation_settlement::counters::SettlementCounters;
use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ReseedCache, WindowReseedCache};
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_writes::initialization::ensure_counter_seeded;
use crate::spend_writes::layers::CounterLayers;

use super::error::SpendIncrementError;
use super::key_team::{key_scope, team_scope};
use super::{SpendIncrementInput, SpendIncrementServices};

// the cost one fan-out pass charges, with the counters the reservation already settled so the direct increment skips them
pub(super) struct ScopeCharge {
    pub(super) cost: Decimal,
    pub(super) reserved: BTreeSet<String>,
    pub(super) request_started_at: Option<DateTime<Utc>>,
}

// one scope's pending increments and its first failure, which never stops its siblings
#[derive(Default)]
pub(super) struct ScopeOutcome {
    pub(super) pending: Vec<PendingSpendIncrement>,
    pub(super) error: Option<SpendIncrementError>,
}

pub(super) type ScopeFuture<'scope> = Pin<Box<dyn Future<Output = ScopeOutcome> + Send + 'scope>>;

// asyncio.gather of the scope coroutines: every scope settles, and the first failure in request order is the one the caller sees
pub(super) async fn run_scopes<Counters, Layers, Rows>(
    input: &SpendIncrementInput<'_>,
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let mut scopes: Vec<ScopeFuture<'_>> = Vec::new();
    if let Some(token) = input.token {
        scopes.push(Box::pin(key_scope(token, charge, services)));
    }
    if let Some(team_id) = input.team_id {
        scopes.push(Box::pin(team_scope(team_id, charge, services)));
    }
    if let Some(user_id) = input.user_id {
        scopes.push(Box::pin(identity_scope(
            user_id,
            input.team_id,
            charge,
            services,
        )));
    }
    if input.end_user_id.is_some() || input.tags.is_some() {
        scopes.push(Box::pin(end_user_and_tag_scope(
            input.end_user_id,
            input.tags.unwrap_or_default(),
            charge,
            services,
        )));
    }
    if let Some(groups) = input
        .model_access_groups
        .filter(|groups| !groups.is_empty())
    {
        scopes.push(Box::pin(model_access_group_scope(groups, charge, services)));
    }
    if input.org_id.is_some() || input.project_id.is_some() {
        scopes.push(Box::pin(org_project_scope(
            input.org_id,
            input.project_id,
            charge,
            services,
        )));
    }
    combine(join_all(scopes).await)
}

// _prepare_spend_counter_increment behind the reserved check: the warm check settles, and the increment waits for the one apply
pub(super) async fn prepare_entity<Counters, Layers, Rows>(
    counter_key: &str,
    source_cache_keys: &[&str],
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    if charge.reserved.contains(counter_key) {
        return ScopeOutcome::default();
    }
    let prepared = ensure_counter_seeded(services.writes, counter_key, source_cache_keys)
        .await
        .map(|()| {
            Some(PendingSpendIncrement {
                counter_key: counter_key.to_owned(),
                increment: charge.cost,
            })
        })
        .map_err(SpendIncrementError::from);
    scope_outcome(prepared)
}

// _team_member_scope then _user_scope: the membership counter needs both ids, and the user counter needs only the user
pub(super) async fn identity_scope<Counters, Layers, Rows>(
    user_id: &str,
    team_id: Option<&str>,
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let mut outcomes = Vec::with_capacity(2);
    if let Some(team_id) = team_id {
        let counter_key = format!("spend:team_member:{user_id}:{team_id}");
        let source_key = CacheKey::TeamMembershipReservation { user_id, team_id }.render();
        outcomes.push(prepare_entity(&counter_key, &[source_key.as_str()], charge, services).await);
    }
    let counter_key = format!("spend:user:{user_id}");
    outcomes.push(prepare_entity(&counter_key, &[user_id], charge, services).await);
    combine(outcomes)
}

// _prepare_end_user_and_tag_spend_increments: the end user first, then every tag once in first-seen order, each with its own outcome
pub(super) async fn end_user_and_tag_scope<Counters, Layers, Rows>(
    end_user_id: Option<&str>,
    tags: &[JsonData],
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let mut outcomes = Vec::new();
    if let Some(end_user_id) = end_user_id {
        let counter_key = format!("spend:end_user:{end_user_id}");
        let source_key = CacheKey::EndUserId(end_user_id).render();
        outcomes.push(prepare_entity(&counter_key, &[source_key.as_str()], charge, services).await);
    }
    for tag in distinct_texts(tags) {
        let counter_key = format!("spend:tag:{tag}");
        let source_key = CacheKey::Tag(&tag).render();
        outcomes.push(prepare_entity(&counter_key, &[source_key.as_str()], charge, services).await);
    }
    combine(outcomes)
}

// _prepare_model_access_group_spend_increments: every group that authorized the request charges its own spend counter
pub(super) async fn model_access_group_scope<Counters, Layers, Rows>(
    groups: &[JsonData],
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let mut outcomes = Vec::new();
    for group in distinct_texts(groups) {
        let counter_key = CacheKey::ModelAccessGroupSpend(&group).render();
        let source_key = CacheKey::ModelAccessGroup(&group).render();
        outcomes.push(prepare_entity(&counter_key, &[source_key.as_str()], charge, services).await);
    }
    combine(outcomes)
}

// _prepare_org_spend_increment then _prepare_project_spend_increment: the budgeted org row is read first, and the project row is its own only floor
pub(super) async fn org_project_scope<Counters, Layers, Rows>(
    org_id: Option<&str>,
    project_id: Option<&str>,
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let mut outcomes = Vec::with_capacity(2);
    if let Some(org_id) = org_id {
        let counter_key = format!("spend:org:{org_id}");
        let with_budget = format!("org_id:{org_id}:with_budget");
        let plain = format!("org_id:{org_id}");
        outcomes.push(
            prepare_entity(
                &counter_key,
                &[with_budget.as_str(), plain.as_str()],
                charge,
                services,
            )
            .await,
        );
    }
    if let Some(project_id) = project_id {
        let counter_key = CacheKey::ProjectSpend(project_id).render();
        let source_key = CacheKey::ProjectId(project_id).render();
        outcomes.push(prepare_entity(&counter_key, &[source_key.as_str()], charge, services).await);
    }
    combine(outcomes)
}

// dict.fromkeys of the nonempty strings: a tag or access group charges once, in first-seen order
pub(super) fn distinct_texts(values: &[JsonData]) -> Vec<String> {
    let mut texts = Vec::new();
    for value in values {
        if let JsonData::String(text) = value
            && !text.is_empty()
        {
            texts.push(text.clone());
        }
    }
    dedup_first(texts)
}

// gather(return_exceptions=True) over the scope's items: every item settles, and the first failure is kept
pub(super) fn combine(outcomes: Vec<ScopeOutcome>) -> ScopeOutcome {
    let mut pending = Vec::new();
    let mut error = None;
    for outcome in outcomes {
        pending.extend(outcome.pending);
        if error.is_none() {
            error = outcome.error;
        }
    }
    ScopeOutcome { pending, error }
}

// one prepared item as a scope outcome, whether it charged, skipped or failed
pub(super) fn scope_outcome(
    prepared: Result<Option<PendingSpendIncrement>, SpendIncrementError>,
) -> ScopeOutcome {
    match prepared {
        Ok(Some(increment)) => ScopeOutcome {
            pending: vec![increment],
            error: None,
        },
        Ok(None) => ScopeOutcome::default(),
        Err(error) => ScopeOutcome {
            pending: Vec::new(),
            error: Some(error),
        },
    }
}

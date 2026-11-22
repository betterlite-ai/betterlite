use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use chrono::{DateTime, Utc};
use futures_util::future::join3;
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
use super::key_team::{WindowScopeKind, window_scope};
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

// one direct counter with the source cache keys its floor falls back to, in the order LiteLLM reads them
pub(super) type CounterItem = (String, Vec<String>);

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
    let key_scope = async {
        match input.token {
            Some(token) => window_scope(WindowScopeKind::Key(token), charge, services).await,
            None => ScopeOutcome::default(),
        }
    };
    let team_scope = async {
        match input.team_id {
            Some(team_id) => window_scope(WindowScopeKind::Team(team_id), charge, services).await,
            None => ScopeOutcome::default(),
        }
    };
    let items = counter_items(input);
    let item_scope = prepare_items(&items, charge, services);
    let (key_outcome, team_outcome, item_outcome) = join3(key_scope, team_scope, item_scope).await;
    combine(vec![key_outcome, team_outcome, item_outcome])
}

// _prepare_spend_counter_increment for every item of a scope: the reserved counters are skipped, the warm checks settle, and the increments wait for the one apply
pub(super) async fn prepare_items<Counters, Layers, Rows>(
    items: &[CounterItem],
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let mut outcomes = Vec::with_capacity(items.len());
    for (counter_key, source_keys) in items {
        if charge.reserved.contains(counter_key) {
            continue;
        }
        let sources: Vec<&str> = source_keys.iter().map(String::as_str).collect();
        let prepared = ensure_counter_seeded(services.writes, counter_key, &sources)
            .await
            .map(|()| {
                Some(PendingSpendIncrement {
                    counter_key: counter_key.clone(),
                    increment: charge.cost,
                })
            })
            .map_err(SpendIncrementError::from);
        outcomes.push(scope_outcome(prepared));
    }
    combine(outcomes)
}

// every direct scope's counters in LiteLLM's request order: team member, user, end user, tags, access groups, org, project
pub(super) fn counter_items(input: &SpendIncrementInput<'_>) -> Vec<CounterItem> {
    let mut items = Vec::new();
    if let Some(user_id) = input.user_id {
        if let Some(team_id) = input.team_id {
            let counter_key = format!("spend:team_member:{user_id}:{team_id}");
            let source_key = CacheKey::TeamMembershipReservation { user_id, team_id }.render();
            items.push((counter_key, vec![source_key]));
        }
        items.push((format!("spend:user:{user_id}"), vec![user_id.to_owned()]));
    }
    if let Some(end_user_id) = input.end_user_id {
        let counter_key = format!("spend:end_user:{end_user_id}");
        let source_key = CacheKey::EndUserId(end_user_id).render();
        items.push((counter_key, vec![source_key]));
    }
    for tag in distinct_texts(input.tags.unwrap_or_default()) {
        items.push(text_item(TextScope::Tag, &tag));
    }
    for group in distinct_texts(input.model_access_groups.unwrap_or_default()) {
        items.push(text_item(TextScope::AccessGroup, &group));
    }
    if let Some(org_id) = input.org_id {
        let counter_key = format!("spend:org:{org_id}");
        let with_budget = format!("org_id:{org_id}:with_budget");
        let plain = format!("org_id:{org_id}");
        items.push((counter_key, vec![with_budget, plain]));
    }
    if let Some(project_id) = input.project_id {
        let counter_key = CacheKey::ProjectSpend(project_id).render();
        let source_key = CacheKey::ProjectId(project_id).render();
        items.push((counter_key, vec![source_key]));
    }
    items
}

// the two list scopes: a tag or an access group charges its own counter, and its source key is the auth cache key it came from
enum TextScope {
    Tag,
    AccessGroup,
}

fn text_item(kind: TextScope, text: &str) -> CounterItem {
    match kind {
        TextScope::Tag => (
            format!("spend:tag:{text}"),
            vec![CacheKey::Tag(text).render()],
        ),
        TextScope::AccessGroup => (
            CacheKey::ModelAccessGroupSpend(text).render(),
            vec![CacheKey::ModelAccessGroup(text).render()],
        ),
    }
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

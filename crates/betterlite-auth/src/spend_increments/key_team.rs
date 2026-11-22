use betterlite_db::key_user_writes::hash::hash_token_if_needed;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;

use crate::budget_counter::CounterEntityType;
use crate::reservation_settlement::counters::SettlementCounters;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ReseedCache, WindowReseedCache};
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_writes::layers::CounterLayers;

use super::SpendIncrementServices;
use super::scopes::{CounterItem, ScopeCharge, ScopeOutcome, combine, prepare_items};
use super::sources::cached_budget_limits;
use super::window::{WindowScope, window_charge, window_outcome};

// the key or team scope a request fans out to: the key token is the only one the sk- safety net can rewrite
pub(super) enum WindowScopeKind<'id> {
    Key(&'id str),
    Team(&'id str),
}

// _key_scope and _team_scope: the counter, the cache key the object's windows come from, and the row identity they charge
pub(super) fn window_scope_of(kind: WindowScopeKind<'_>) -> WindowScope {
    match kind {
        WindowScopeKind::Key(token) => {
            let key_token = hash_token_if_needed(token);
            WindowScope {
                counter_key: format!("spend:key:{key_token}"),
                source_key: key_token.clone(),
                entity_type: CounterEntityType::Key,
                entity_id: key_token,
            }
        }
        WindowScopeKind::Team(team_id) => WindowScope {
            counter_key: format!("spend:team:{team_id}"),
            source_key: format!("team_id:{team_id}"),
            entity_type: CounterEntityType::Team,
            entity_id: team_id.to_owned(),
        },
    }
}

// a key or team scope: its counter first, then the windows of the object cached under its source key, even when the counter is reserved
pub(super) async fn window_scope<Counters, Layers, Rows>(
    kind: WindowScopeKind<'_>,
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let scope = window_scope_of(kind);
    let item: CounterItem = (scope.counter_key.clone(), vec![scope.source_key.clone()]);
    let items = [item];
    let mut outcome = prepare_items(&items, charge, services).await;
    if outcome.error.is_some() {
        return outcome;
    }
    let windows = match cached_budget_limits(services.writes, &scope.source_key).await {
        Ok(windows) => windows,
        Err(error) => {
            return ScopeOutcome {
                pending: Vec::new(),
                error: Some(error),
            };
        }
    };
    let now = services.writes.reseed.clock.now();
    let durations = services.writes.reseed.durations.as_ref();
    let mut outcomes = Vec::new();
    for window in &windows {
        if let Some(facts) = window_charge(&scope, window, charge, now, durations) {
            outcomes.push(window_outcome(&facts, charge, services).await);
        }
    }
    let windows_outcome = combine(outcomes);
    outcome.pending.extend(windows_outcome.pending);
    ScopeOutcome {
        pending: outcome.pending,
        error: windows_outcome.error,
    }
}

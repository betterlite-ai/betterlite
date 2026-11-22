use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::key_user_writes::hash::hash_token_if_needed;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;

use crate::budget_counter::CounterEntityType;
use crate::reservation_settlement::counters::SettlementCounters;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ReseedCache, WindowReseedCache};
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_writes::layers::CounterLayers;

use super::SpendIncrementServices;
use super::scopes::{ScopeCharge, ScopeOutcome, combine, prepare_entity};
use super::sources::cached_budget_limits;
use super::window::{WindowScope, window_charge, window_outcome};

// _key_scope: the "sk-" safety net hashes a raw token once, and the hashed token is both the counter id and its source cache key
pub(super) async fn key_scope<Counters, Layers, Rows>(
    token: &str,
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let key_token = hash_token_if_needed(token);
    window_capable_scope(
        WindowScope {
            counter_key: format!("spend:key:{key_token}"),
            source_key: key_token.clone(),
            entity_type: CounterEntityType::Key,
            entity_id: key_token,
        },
        charge,
        services,
    )
    .await
}

// _team_scope: the team object is cached under team_id, which is also the counter's only source floor
pub(super) async fn team_scope<Counters, Layers, Rows>(
    team_id: &str,
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    window_capable_scope(
        WindowScope {
            counter_key: format!("spend:team:{team_id}"),
            source_key: format!("team_id:{team_id}"),
            entity_type: CounterEntityType::Team,
            entity_id: team_id.to_owned(),
        },
        charge,
        services,
    )
    .await
}

// a key or team scope, whose cached object still charges its windows when the counter itself is reserved
async fn window_capable_scope<Counters, Layers, Rows>(
    scope: WindowScope,
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let prepared = prepare_entity(
        &scope.counter_key,
        &[scope.source_key.as_str()],
        charge,
        services,
    )
    .await;
    if prepared.error.is_some() {
        return prepared;
    }
    let mut pending = prepared.pending;
    let windows = match cached_budget_limits(services.writes, &scope.source_key).await {
        Ok(windows) => windows,
        Err(error) => {
            return ScopeOutcome {
                pending: Vec::new(),
                error: Some(error),
            };
        }
    };
    let windows_outcome = run_windows(&scope, &windows, charge, services).await;
    pending.extend(windows_outcome.pending);
    ScopeOutcome {
        pending,
        error: windows_outcome.error,
    }
}

// asyncio.gather of one scope's window coroutines: one bad window fails alone, after the apply, like any scope error
async fn run_windows<Counters, Layers, Rows>(
    scope: &WindowScope,
    windows: &[JsonData],
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let now = services.writes.reseed.clock.now();
    let durations = services.writes.reseed.durations.as_ref();
    let mut outcomes = Vec::new();
    for window in windows {
        if let Some(facts) = window_charge(scope, window, charge, now, durations) {
            outcomes.push(window_outcome(&facts, charge, services).await);
        }
    }
    combine(outcomes)
}

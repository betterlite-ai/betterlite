use std::sync::{Arc, Mutex};

use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_router::budget_increment::dual::DualWindowCache;
use betterlite_router::budget_increment::increment::BudgetIncrement;
use betterlite_router::budget_increment::window::WindowRedis;
use betterlite_router::budget_windows::service::BudgetWindows;
use betterlite_router::routing_budget_state::service::RoutingBudgetState;
use betterlite_router::routing_budget_state::spend_redis::SpendRedis;
use betterlite_router::routing_value_store::Clock;
use tokio::task::JoinSet;
use tokio_util::task::TaskTracker;

mod tests;

// B020 installs the state this assembles, starts its periodic sync and calls its shutdown flush
#[must_use]
pub fn routing_budget_state<R: WindowRedis + SpendRedis + 'static>(
    windows: BudgetWindows,
    memory: Arc<Mutex<LocalTtlCache>>,
    redis: R,
    clock: Clock,
) -> Arc<RoutingBudgetState<R>> {
    let cache = Arc::new(DualWindowCache {
        memory,
        redis: Some(redis),
    });
    Arc::new(RoutingBudgetState {
        increments: BudgetIncrement {
            cache,
            clock,
            queue: Arc::default(),
            flush_lock: Arc::default(),
        },
        windows,
        sync: Mutex::new(JoinSet::new()),
        flushes: TaskTracker::new(),
    })
}

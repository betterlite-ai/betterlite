#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::routing_state as state_config;
use betterlite_config::sections::usage_v2_accounting as config;

use super::resolve;

// the strategy's own 0.1 s interval beats the generic DEFAULT_REDIS_SYNC_INTERVAL, and its writes batch
#[test]
fn resolves_the_counter_loop_the_v2_strategy_batches_with() {
    let resolved = resolve(
        &config::Settings {
            ttl: Duration::from_secs(45),
        },
        &state_config::Settings {
            default_sync_interval: Duration::from_secs(3),
        },
    );
    assert_eq!(resolved.ttl, Duration::from_secs(45));
    assert_eq!(
        resolved.counters.sync_interval,
        Some(Duration::from_millis(100))
    );
    assert_eq!(
        resolved.counters.resolved_sync_interval(),
        Duration::from_millis(100)
    );
    assert!(resolved.counters.should_batch_redis_writes);
}

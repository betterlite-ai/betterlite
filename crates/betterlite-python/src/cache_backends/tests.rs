#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_router::redis_failure_policy::RedisFailurePolicy;
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::redis_failure_policy::gate::PolicyGate;
use pyo3::prelude::*;

use super::register;

// A real breaker for the tests that only need a cache to build; the breaker tests build their own.
pub(super) fn test_gate() -> Arc<PolicyGate> {
    let settings = BreakerSettings {
        enabled: true,
        failure_threshold: 5,
        recovery_timeout: Duration::from_secs(60),
        timeout_min_duration: Duration::from_secs(5),
        timeout_log_interval: Duration::from_secs(5),
    };
    Arc::new(PolicyGate::new(
        RedisFailurePolicy::new(settings, BreakerMetrics::new(None)),
        Arc::new(|| Duration::ZERO),
    ))
}

#[test]
fn register_adds_the_backend_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeMemory").unwrap());
        assert!(module.hasattr("NativeDisk").unwrap());
        assert!(module.hasattr("NativeRedis").unwrap());
        assert!(module.hasattr("NativeDual").unwrap());
        assert!(module.hasattr("NativeDeferred").unwrap());
        assert!(module.hasattr("NativeDeclaredBatchRead").unwrap());
        assert!(module.hasattr("NativeRedisBatch").unwrap());
        assert!(module.hasattr("RedisCircuitBreakerOpenError").unwrap());
    });
}

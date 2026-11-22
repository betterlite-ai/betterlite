#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::rate_limiter_settings;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use tokio::sync::Mutex as AsyncMutex;

use super::{AdmissionLimiter as _, AdmissionLimiterError, AssemblyLimiter};
use crate::concurrency_gauges::ConcurrencyGauges;
use crate::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use crate::dynamic_admission::tests::support::unverifiable;
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_assembly::RateLimiterAssembly;
use crate::limiter_records::{CacheCounterValue, LimiterClock, ModelGroupResolver};
use crate::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind, WindowStore};
use crate::settlement_and_rollback::SettlementError;
use crate::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use crate::windowed_admission::{UuidSlotIds, WindowedAdmission, WindowedAdmissionError};

struct FixedClock;

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .expect("the fixed clock stays representable")
    }
}

struct NoCounterCache;

#[async_trait::async_trait]
impl CounterCache for NoCounterCache {
    async fn get_cache(&self, _key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        None
    }

    async fn set_cache(&self, _key: &str, _value: CacheCounterValue, _ttl: i64, _local_only: bool) {
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

struct NoGaugeCache;

#[async_trait::async_trait]
impl GaugeCache for NoGaugeCache {
    async fn get(&self, _key: &str, _local_only: bool) -> Option<GaugeCacheValue> {
        None
    }

    async fn set(&self, _key: &str, _value: GaugeCacheValue, _ttl: Duration, _local_only: bool) {}

    async fn batch_get(
        &self,
        _keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        None
    }
}

struct NoUsage;

#[async_trait::async_trait]
impl UsageIncrements for NoUsage {
    async fn increment(&self, _operation: &IncrementOperation) {}

    async fn increment_pipeline(&self, _operations: &[IncrementOperation]) {}

    async fn set_local_counter(&self, _key: &str, _value: f64, _ttl: Option<i64>) {}
}

struct NoGroups;

impl ModelGroupResolver for NoGroups {
    fn resolve(&self, _model: &str) -> Option<String> {
        None
    }
}

fn window_store() -> WindowStore {
    WindowStore {
        cache: Arc::new(NoCounterCache),
        rate_limiter_script: None,
        counter_read_script: None,
        request_batch: None,
        check_and_increment_lock: AsyncMutex::new(()),
    }
}

fn concurrency_gauges(
    clock: Arc<dyn LimiterClock>,
    fail_closed: FailClosedPolicy,
) -> ConcurrencyGauges {
    ConcurrencyGauges {
        cache: Arc::new(NoGaugeCache),
        clock,
        acquire_script: None,
        release_script: None,
        count_script: None,
        release_batch: None,
        fail_closed,
        check_and_increment_lock: AsyncMutex::new(()),
    }
}

fn windowed_admission(
    window_size: i64,
    clock: Arc<dyn LimiterClock>,
    fail_closed: FailClosedPolicy,
    windows: Arc<WindowStore>,
    gauges: Arc<ConcurrencyGauges>,
) -> WindowedAdmission {
    WindowedAdmission {
        window_size,
        clock,
        fail_closed,
        windows,
        refund_script: None,
        gauges,
        slot_ids: Arc::new(UuidSlotIds),
    }
}

fn token_adjustments() -> TokenAdjustments {
    TokenAdjustments {
        windows: window_store(),
        usage: Arc::new(NoUsage),
        post_call_batch: None,
        token_increment_script: None,
        guarded_token_script: None,
    }
}

fn build_assembly(window_size: i64) -> RateLimiterAssembly {
    let clock: Arc<dyn LimiterClock> = Arc::new(FixedClock);
    let fail_closed = FailClosedPolicy { enabled: false };
    let windows = Arc::new(window_store());
    let gauges = Arc::new(concurrency_gauges(Arc::clone(&clock), fail_closed));
    let admission = Arc::new(windowed_admission(
        window_size,
        Arc::clone(&clock),
        fail_closed,
        Arc::clone(&windows),
        Arc::clone(&gauges),
    ));
    let adjustments = Arc::new(token_adjustments());
    RateLimiterAssembly {
        settings: rate_limiter_settings::Settings {
            fail_closed: false,
            window_size,
            tpm_reservation_enabled: false,
        },
        clock,
        fail_closed,
        windows,
        check_and_increment_script: None,
        gauges,
        admission,
        adjustments,
        model_groups: Box::new(NoGroups),
        batch_limiter: Mutex::new(None),
    }
}

fn assembly_limiter(window_size: i64) -> AssemblyLimiter {
    AssemblyLimiter {
        assembly: Arc::new(build_assembly(window_size)),
    }
}

#[test]
fn limiter_port_assembly_limiter_reports_the_assembly_window_size() {
    assert_eq!(assembly_limiter(120).window_size(), 120);
}

#[test]
fn limiter_port_from_converts_the_unverifiable_error() {
    let converted = AdmissionLimiterError::from(unverifiable());
    assert!(matches!(
        converted,
        AdmissionLimiterError::Unverifiable(error) if error.status_code == 503
    ));
}

#[test]
fn limiter_port_from_settlement_keeps_the_fail_closed_error() {
    let flattened =
        AdmissionLimiterError::from_settlement(SettlementError::Unverifiable(unverifiable()));
    assert!(matches!(
        flattened,
        AdmissionLimiterError::Unverifiable(error) if error.status_code == 503
    ));
}

#[test]
fn limiter_port_from_settlement_keeps_other_errors() {
    let flattened = AdmissionLimiterError::from_settlement(SettlementError::DescriptorCount {
        descriptors: 2,
        increments: 1,
    });
    assert!(matches!(
        flattened,
        AdmissionLimiterError::Settlement(SettlementError::DescriptorCount { .. })
    ));
}

#[test]
fn limiter_port_from_windowed_keeps_the_fail_closed_error() {
    let flattened =
        AdmissionLimiterError::from_windowed(WindowedAdmissionError::Unverifiable(unverifiable()));
    assert!(matches!(
        flattened,
        AdmissionLimiterError::Unverifiable(error) if error.status_code == 503
    ));
}

#[test]
fn limiter_port_from_windowed_reaches_the_settlement_inside() {
    let flattened = AdmissionLimiterError::from_windowed(WindowedAdmissionError::Settlement(
        SettlementError::Unverifiable(unverifiable()),
    ));
    assert!(matches!(
        flattened,
        AdmissionLimiterError::Unverifiable(error) if error.status_code == 503
    ));
}

#[test]
fn limiter_port_from_windowed_keeps_the_settlement_error() {
    let flattened = AdmissionLimiterError::from_windowed(WindowedAdmissionError::Settlement(
        SettlementError::DescriptorCount {
            descriptors: 2,
            increments: 1,
        },
    ));
    assert!(matches!(
        flattened,
        AdmissionLimiterError::Settlement(SettlementError::DescriptorCount { .. })
    ));
}

#[test]
fn limiter_port_from_windowed_keeps_other_errors() {
    let flattened = AdmissionLimiterError::from_windowed(WindowedAdmissionError::WindowPairCount {
        keys: 1,
        values: 2,
    });
    assert!(matches!(
        flattened,
        AdmissionLimiterError::Windowed(WindowedAdmissionError::WindowPairCount { .. })
    ));
}

#[tokio::test]
async fn limiter_port_assembly_limiter_settles_no_descriptors_as_ok() {
    let response = assembly_limiter(60)
        .atomic_check_and_increment_by_n(&[], &[], None)
        .await
        .expect("no descriptors is nothing to settle");
    assert_eq!(response.overall_code, "OK");
    assert!(response.statuses.is_empty());
}

#[tokio::test]
async fn limiter_port_assembly_limiter_checks_no_descriptors_as_ok() {
    let response = assembly_limiter(60)
        .should_rate_limit(&[], None, false)
        .await
        .expect("no descriptors is nothing to check");
    assert_eq!(response.overall_code, "OK");
    assert!(response.statuses.is_empty());
}

#[tokio::test]
async fn limiter_port_assembly_limiter_reads_without_a_redis_script() {
    let response = assembly_limiter(60)
        .should_rate_limit(&[], None, true)
        .await
        .expect("a read-only check without descriptors is nothing to check");
    assert_eq!(response.overall_code, "OK");
    assert!(response.statuses.is_empty());
}

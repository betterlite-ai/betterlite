use std::time::Duration;

use betterlite_error::rate_limit::RateLimitCategory;
use rust_decimal::Decimal;

use super::support::{Harness, RPM_KEY, TPM_KEY, at, deployment, payload, rpm, success_event};
use crate::router_observations::usage::DeploymentUsage;
use crate::routing_state::Settings;
use crate::usage_v2_accounting::deployment::DeploymentFacts;
use crate::usage_v2_accounting::error::RpmLimitExceeded;
use crate::usage_v2_accounting::{SHOULD_BATCH_WRITES, SYNC_INTERVAL};

const OTHER_RPM_KEY: &str = "dep-b:gpt-4o:rpm:00-00";

pub(super) async fn run(name: &str) {
    // the counter-loop settings need no runtime
    if name == "counter_loop_uses_the_v2_interval_and_batching" {
        counter_loop_uses_the_v2_interval_and_batching();
        return;
    }
    match name {
        "local_boundary_rejects_the_count_at_the_limit" => {
            local_boundary_rejects_the_count_at_the_limit().await;
        }
        "sync_facade_uses_the_same_local_boundary" => {
            sync_facade_uses_the_same_local_boundary().await;
        }
        "async_rejection_carries_the_retry_header" => {
            async_rejection_carries_the_retry_header().await;
        }
        "success_zero_tokens_still_writes_the_key" => {
            success_zero_tokens_still_writes_the_key().await;
        }
        "window_ttl_expires_the_rpm_key" => window_ttl_expires_the_rpm_key().await,
        "shared_counts_do_not_decide_the_local_boundary" => {
            shared_counts_do_not_decide_the_local_boundary().await;
        }
        "rpm_storage_error_fails_open" => rpm_storage_error_fails_open().await,
        "the_v2_counter_loop_starts_and_stops" => the_v2_counter_loop_starts_and_stops().await,
        other => panic!("unknown case {other}"),
    }
}

async fn local_boundary_rejects_the_count_at_the_limit() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(2)));
    harness.seed_local(RPM_KEY, 1);
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    assert_eq!(harness.local(RPM_KEY), Some(2));
    let at_limit = harness
        .accounting
        .pre_call(&facts, at(0))
        .await
        .expect_err("the count at the limit is rejected");
    assert_eq!(at_limit.usage, Decimal::from(2));
    assert_eq!(harness.local(RPM_KEY), Some(2));
    harness.seed_local(OTHER_RPM_KEY, 3);
    let over = deployment("dep-b", rpm(Some(2)));
    let rejected = harness
        .accounting
        .pre_call(&over, at(0))
        .await
        .expect_err("a count past the limit is rejected");
    assert_eq!(rejected.usage, Decimal::from(3));
    assert_eq!(harness.local(OTHER_RPM_KEY), Some(3));
}

async fn sync_facade_uses_the_same_local_boundary() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(1)));
    assert_eq!(harness.accounting.pre_call_now(&facts, at(0)), Ok(()));
    assert_eq!(harness.local(RPM_KEY), Some(1));
    let error = harness
        .accounting
        .pre_call_now(&facts, at(0))
        .expect_err("the count at the limit is rejected");
    assert_eq!(error.limit, 1);
    assert_eq!(error.usage, Decimal::ONE);
    assert_eq!(error.retry_after_seconds, None);
    assert_eq!(error.num_retries, None);
    assert_eq!(harness.local(RPM_KEY), Some(1));
}

async fn async_rejection_carries_the_retry_header() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(0)));
    let error = harness
        .accounting
        .pre_call(&facts, at(0))
        .await
        .expect_err("an rpm of zero rejects the request");
    assert_eq!(error.retry_after_seconds, Some(60));
    assert_eq!(error.num_retries, Some(2));
    assert_eq!(error.model, "gpt-4o");
    assert_eq!(RpmLimitExceeded::STATUS_CODE, 429);
    assert_eq!(RpmLimitExceeded::CODE, "429");
    assert_eq!(RpmLimitExceeded::ERROR_TYPE, "throttling_error");
    assert_eq!(
        RpmLimitExceeded::CATEGORY,
        RateLimitCategory::VendorRateLimit
    );
}

async fn success_zero_tokens_still_writes_the_key() {
    let harness = Harness::new();
    let event = success_event(payload(
        Some("gpt"),
        Some("dep-a"),
        Some("openai/gpt-4o-mini"),
        0,
    ));
    assert_eq!(
        harness
            .accounting
            .log_success(&event, at(0))
            .await
            .as_deref(),
        Some(TPM_KEY)
    );
    assert_eq!(harness.local(TPM_KEY), Some(0));
    assert_eq!(
        harness.accounting.log_success_now(&event, at(0)).as_deref(),
        Some(TPM_KEY)
    );
    assert_eq!(harness.local(TPM_KEY), Some(0));
}

async fn window_ttl_expires_the_rpm_key() {
    let harness = Harness::new();
    let facts = deployment("dep-a", DeploymentUsage::default());
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    harness.set_millis(60_000);
    assert_eq!(harness.local(RPM_KEY), Some(1));
    harness.set_millis(60_001);
    assert_eq!(harness.local(RPM_KEY), None);
}

async fn shared_counts_do_not_decide_the_local_boundary() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(2)));
    harness.store.seed(RPM_KEY, Decimal::from(5));
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    assert_eq!(harness.local(RPM_KEY), Some(1));
    assert_eq!(harness.accounting.pre_call_now(&facts, at(0)), Ok(()));
    assert_eq!(harness.local(RPM_KEY), Some(2));
    assert!(harness.accounting.counters.sync_with_redis().await);
    assert_eq!(harness.store.value(RPM_KEY), Decimal::from(7));
    assert_eq!(harness.local(RPM_KEY), Some(7));
}

async fn rpm_storage_error_fails_open() {
    let harness = Harness::new();
    let model_id = "d".repeat(1_100);
    let facts = DeploymentFacts {
        model_id: &model_id,
        model: "gpt-4o",
        limits: rpm(Some(1)),
        num_retries: Some(2),
    };
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    assert_eq!(harness.accounting.pre_call_now(&facts, at(0)), Ok(()));
    assert!(harness.accounting.counters.get_tracked_keys().is_empty());
}

// BaseRoutingStrategy.__init__ starts the sync task; cancel_sync_task and cleanup stop it again
async fn the_v2_counter_loop_starts_and_stops() {
    let harness = Harness::new();
    harness.accounting.counters.start();
    harness.accounting.counters.cancel_sync_task();
    let stopped = tokio::time::timeout(
        Duration::from_secs(1),
        harness.accounting.counters.cleanup(),
    )
    .await;
    assert!(stopped.is_ok(), "cleanup did not stop the v2 sync task");
}

fn counter_loop_uses_the_v2_interval_and_batching() {
    assert_eq!(SYNC_INTERVAL, Duration::from_millis(100));
    assert!(SHOULD_BATCH_WRITES);
    let settings = Settings {
        sync_interval: Some(SYNC_INTERVAL),
        default_sync_interval: Duration::from_secs(60),
        should_batch_redis_writes: SHOULD_BATCH_WRITES,
    };
    assert_eq!(
        settings.resolved_sync_interval(),
        Duration::from_millis(100)
    );
}

use std::time::Duration;

use betterlite_error::rate_limit::RateLimitCategory;
use rust_decimal::Decimal;
use tokio::time::{sleep, timeout};

use super::support::{Harness, RPM_KEY, TPM_KEY, at, deployment, payload, rpm, success_event};
use crate::router_observations::usage::DeploymentUsage;
use crate::routing_state::Settings;
use crate::usage_v2_accounting::deployment::DeploymentFacts;
use crate::usage_v2_accounting::error::RpmLimitExceeded;
use crate::usage_v2_accounting::{SHOULD_BATCH_WRITES, SYNC_INTERVAL};

const OTHER_RPM_KEY: &str = "dep-b:gpt-4o:rpm:00-00";
const SYNC_FACADE_RPM_KEY: &str = "dep-c:gpt-4o:rpm:00-00";
const SYNC_CALLBACK_TPM_KEY: &str = "dep-b:openai/gpt-4o-mini:tpm:00-00";

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
        "sync_facade_uses_the_same_local_boundary" => sync_facade_uses_the_same_local_boundary(),
        "async_rejection_carries_the_retry_header" => {
            async_rejection_carries_the_retry_header().await;
        }
        "success_zero_tokens_still_writes_the_key" => {
            success_zero_tokens_still_writes_the_key().await;
        }
        "window_ttl_expires_the_rpm_key" => window_ttl_expires_the_rpm_key().await,
        "configured_ttl_expires_every_window_key" => {
            configured_ttl_expires_every_window_key().await;
        }
        "shared_counts_do_not_decide_the_local_boundary" => {
            shared_counts_do_not_decide_the_local_boundary().await;
        }
        "rpm_storage_error_fails_open" => rpm_storage_error_fails_open().await,
        "the_v2_counter_loop_starts_and_stops" => the_v2_counter_loop_starts_and_stops().await,
        "sync_pre_call_counts_the_request_before_rejecting_it" => {
            sync_pre_call_counts_the_request_before_rejecting_it();
        }
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
    assert_eq!(at_limit.usage, Decimal::from(2_i64));
    assert_eq!(harness.local(RPM_KEY), Some(2));
    harness.seed_local(OTHER_RPM_KEY, 3);
    let over = deployment("dep-b", rpm(Some(2)));
    let rejected = harness
        .accounting
        .pre_call(&over, at(0))
        .await
        .expect_err("a count past the limit is rejected");
    assert_eq!(rejected.usage, Decimal::from(3_i64));
    assert_eq!(harness.local(OTHER_RPM_KEY), Some(3));
}

fn sync_facade_uses_the_same_local_boundary() {
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
    // a deployment without num_retries still sends the retry header and leaves num_retries empty
    let no_retries = DeploymentFacts {
        model_id: "dep-b",
        model: "gpt-4o",
        limits: rpm(Some(0)),
        num_retries: None,
    };
    let absent = harness
        .accounting
        .pre_call(&no_retries, at(0))
        .await
        .expect_err("an rpm of zero rejects the request");
    assert_eq!(absent.retry_after_seconds, Some(60));
    assert_eq!(absent.num_retries, None);
}

async fn success_zero_tokens_still_writes_the_key() {
    let harness = Harness::new();
    let mut event = success_event(payload(
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
    // a success event without a call type is not a batch retrieve, so the sync callback counts it
    event.call_type = None;
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

// the configured ttl, not a built-in minute, expires every key the pre-call and success paths write
async fn configured_ttl_expires_every_window_key() {
    let harness = Harness::with_ttl(Duration::from_secs(120));
    let facts = deployment("dep-a", DeploymentUsage::default());
    let sync_facts = deployment("dep-c", DeploymentUsage::default());
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    assert_eq!(harness.accounting.pre_call_now(&sync_facts, at(0)), Ok(()));
    let model = Some("openai/gpt-4o-mini");
    let async_event = success_event(payload(Some("gpt"), Some("dep-a"), model, 7));
    let sync_event = success_event(payload(Some("gpt"), Some("dep-b"), model, 7));
    assert_eq!(
        harness
            .accounting
            .log_success(&async_event, at(0))
            .await
            .as_deref(),
        Some(TPM_KEY)
    );
    assert_eq!(
        harness
            .accounting
            .log_success_now(&sync_event, at(0))
            .as_deref(),
        Some(SYNC_CALLBACK_TPM_KEY)
    );
    harness.set_millis(60_001);
    assert_eq!(harness.local(RPM_KEY), Some(1));
    assert_eq!(harness.local(SYNC_FACADE_RPM_KEY), Some(1));
    assert_eq!(harness.local(TPM_KEY), Some(7));
    assert_eq!(harness.local(SYNC_CALLBACK_TPM_KEY), Some(7));
    harness.set_millis(120_001);
    assert_eq!(harness.local(RPM_KEY), None);
    assert_eq!(harness.local(SYNC_FACADE_RPM_KEY), None);
    assert_eq!(harness.local(TPM_KEY), None);
    assert_eq!(harness.local(SYNC_CALLBACK_TPM_KEY), None);
}

async fn shared_counts_do_not_decide_the_local_boundary() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(2)));
    harness.store.seed(RPM_KEY, Decimal::from(5_i64));
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    assert_eq!(harness.local(RPM_KEY), Some(1));
    assert_eq!(harness.accounting.pre_call_now(&facts, at(0)), Ok(()));
    assert_eq!(harness.local(RPM_KEY), Some(2));
    assert!(harness.accounting.counters.sync_with_redis().await);
    assert_eq!(harness.store.value(RPM_KEY), Decimal::from(7_i64));
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

// BaseRoutingStrategy.setup_sync_task: a started loop flushes on its tick and a cancelled one stops
async fn the_v2_counter_loop_starts_and_stops() {
    let harness = Harness::with_sync_interval(SYNC_INTERVAL);
    let facts = deployment("dep-a", rpm(Some(2)));
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    harness.accounting.counters.start();
    let flushed = timeout(Duration::from_secs(2), async {
        while harness.store.value(RPM_KEY) == Decimal::ZERO {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        flushed.is_ok(),
        "the started loop never flushed the queued increment"
    );
    harness.accounting.counters.cancel_sync_task();
    assert_eq!(harness.accounting.pre_call(&facts, at(0)).await, Ok(()));
    sleep(Duration::from_millis(300)).await;
    assert_eq!(harness.store.value(RPM_KEY), Decimal::ONE);
    harness.accounting.counters.cleanup().await;
}

// pre_call_check counts the sync request first and rejects it once the updated count is past the limit
fn sync_pre_call_counts_the_request_before_rejecting_it() {
    let harness = Harness::new();
    let facts = deployment("dep-a", rpm(Some(0)));
    let error = harness
        .accounting
        .pre_call_now(&facts, at(0))
        .expect_err("an rpm of zero rejects the first request");
    assert_eq!(error.limit, 0);
    assert_eq!(error.usage, Decimal::ONE);
    assert_eq!(error.retry_after_seconds, None);
    assert_eq!(error.num_retries, None);
    assert_eq!(harness.local(RPM_KEY), Some(1));
}

fn counter_loop_uses_the_v2_interval_and_batching() {
    assert_eq!(SYNC_INTERVAL, Duration::from_millis(100));
    let settings = Settings {
        sync_interval: Some(SYNC_INTERVAL),
        default_sync_interval: Duration::from_secs(60),
        should_batch_redis_writes: SHOULD_BATCH_WRITES,
    };
    assert_eq!(
        settings.resolved_sync_interval(),
        Duration::from_millis(100)
    );
    assert!(settings.should_batch_redis_writes);
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::Response;
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::{
    BUDGET_UNVERIFIABLE_DETAIL, CurrentSpend, CurrentSpendError, SpendQuery, WindowDescriptors,
};
use betterlite_auth::model_budget_limiter::clock::SystemClock as UptimeClock;
use betterlite_auth::spend_counter_batch::scope::SpendCounterScope;
use betterlite_auth::spend_reseed::cache::ReseedCache as _;
use betterlite_auth::spend_reseed::locks::LockRegistry;
use betterlite_auth::spend_reseed::reseed::SpendCounterReseed;
use betterlite_auth::spend_reseed::sql::SqlSpendCounterRows;
use betterlite_auth::spend_reseed::values::SpendValueError;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_config::loader::ConfigLoader;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_models::catalog::BundledCatalog;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use super::spend_failure_exception;
use crate::auth::auth_resource_grants::denial_response;
use crate::state::current_spend::Slot;
use crate::state::shared::Shared;

const COUNTER: &str = "spend:key:hashed";

struct NoDurations;

impl DurationParser for NoDurations {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

fn slot(source: &str) -> Slot {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("the test config loads");
    let shared = Shared {
        bundled_catalog: BundledCatalog::default().get(),
    };
    Slot::new(&config, &shared)
}

fn reseed_without_database() -> SpendCounterReseed<SqlSpendCounterRows> {
    SpendCounterReseed {
        rows: None,
        locks: LockRegistry::from(1_000_i64),
        clock: Box::new(UptimeClock {
            started: Instant::now(),
        }),
        durations: Arc::new(NoDurations),
        tasks: TaskTracker::new(),
        gate: Arc::new(LoopBoundSemaphore {
            value: 25,
            slot: Mutex::new(None),
        }),
        stall_tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        deadline_seconds: 5.0_f64,
    }
}

async fn seeded_cache() -> DualCache<LocalMemoryOps> {
    let memory = LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)).expect("the memory tier");
    let settings = DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1_000,
        redis_delete_chunk_size: 1_000,
    };
    let cache = DualCache::new(Arc::new(memory), None, settings);
    cache
        .memory_set(COUNTER, Decimal::from(5_i32))
        .await
        .expect("the per-pod counter");
    cache
}

async fn body_text(response: Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    String::from_utf8(body.to_vec()).expect("the response body is UTF-8")
}

#[tokio::test]
async fn production_service_and_http503() {
    let strict = slot("general_settings: {fail_closed_budget_enforcement: true}");
    assert!(strict.fail_closed_budget_enforcement);
    assert!(!slot("model_list: []").fail_closed_budget_enforcement);

    let cache = seeded_cache().await;
    let batch = SpendCounterScope::default();
    let reseed = reseed_without_database();
    let service = CurrentSpend {
        cache: &cache,
        batch: &batch,
        reseed: &reseed,
        fail_closed_budget_enforcement: strict.fail_closed_budget_enforcement,
    };
    let query = SpendQuery {
        counter_key: COUNTER,
        fallback_spend: Decimal::ZERO,
        max_budget: Some(Decimal::TEN),
        window: WindowDescriptors {
            entity_type: None,
            entity_id: None,
            duration: None,
            start: None,
        },
        fallback_authoritative: false,
    };
    let error = service.get_current_spend(&query).await.unwrap_err();
    let response = denial_response(&spend_failure_exception(&error));
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_text(response).await;
    assert!(
        body.contains(&format!("\"message\":\"{BUDGET_UNVERIFIABLE_DETAIL}\"")),
        "{body}"
    );
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"503\""), "{body}");
}

#[tokio::test]
async fn other_spend_failures_answer_401_auth_error() {
    let error =
        CurrentSpendError::<fmt::Error>::Value(SpendValueError::NotNumeric { kind: "a list" });
    let response = denial_response(&spend_failure_exception(&error));
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = body_text(response).await;
    assert!(
        body.contains("\"message\":\"Authentication Error, "),
        "{body}"
    );
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
}

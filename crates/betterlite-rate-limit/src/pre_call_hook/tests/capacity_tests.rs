#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use super::support::{
    self, Fixture, Parts, count, entries, occupied_registry, parallel_key, request_window,
    token_window,
};
use crate::pre_call_hook::{PreCallHookError, PreCallHookRejection};

fn limited_auth() -> UserAPIKeyAuth {
    support::auth(r#"{"api_key": "key-1", "rpm_limit": 10, "max_parallel_requests": 2}"#)
}

#[tokio::test]
async fn capacity_charges_one_request() {
    let fixture = Fixture::plain();
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 5}"#);

    let guard = fixture
        .hook
        .request_capacity(&auth, "gpt4", None)
        .await
        .expect("the capacity admission passes");
    drop(guard);
    fixture.settle().await;

    assert_eq!(
        fixture
            .recorder
            .counter(&support::requests_key("api_key", "key-1")),
        Some(count(1))
    );
}

#[tokio::test]
async fn capacity_skips_the_token_check() {
    let fixture = Fixture::new(&Parts::new(
        entries([token_window("api_key", "key-1", 5)]),
        Vec::new(),
        None,
    ));
    let auth = support::auth(r#"{"api_key": "key-1", "tpm_limit": 5}"#);

    let guard = fixture
        .hook
        .request_capacity(&auth, "gpt4", None)
        .await
        .expect("the capacity admission skips the token counter");
    drop(guard);
    fixture.settle().await;
}

#[tokio::test]
async fn capacity_checks_the_model_limit_from_its_argument() {
    let mut parts = Parts::new(
        entries([request_window("model_per_key", "key-1:gpt-4", 1)]),
        Vec::new(),
        None,
    );
    parts.groups = vec![(String::from("gpt4"), String::from("gpt-4"))];
    let fixture = Fixture::new(&parts);
    let auth =
        support::auth(r#"{"api_key": "key-1", "metadata": {"model_rpm_limit": {"gpt-4": 1}}}"#);

    let Err(error) = fixture.hook.request_capacity(&auth, "gpt4", None).await else {
        panic!("the model limit rejects the capacity admission");
    };

    assert!(matches!(&error, PreCallHookError::RateLimited(_)));
    fixture.settle().await;
}

#[tokio::test]
async fn capacity_raises_on_an_over_limit_admission_and_keeps_the_slots() {
    let fixture = Fixture::new(&Parts::new(
        entries([request_window("api_key", "key-1", 0)]),
        vec![(
            parallel_key("api_key", "key-1"),
            occupied_registry(&["old-1", "old-2", "slot-1"]),
        )],
        None,
    ));
    let auth = limited_auth();
    let key = parallel_key("api_key", "key-1");

    let Err(error) = fixture.hook.request_capacity(&auth, "gpt4", None).await else {
        panic!("the parallel request gauge is full");
    };

    assert!(matches!(&error, PreCallHookError::RateLimited(_)));
    let Some(PreCallHookRejection::RateLimited(rate_limited)) = error.rejection() else {
        panic!("the rejection is a 429");
    };
    assert!(
        rate_limited
            .message
            .starts_with("Rate limit exceeded for api_key: key-1")
    );
    fixture.settle().await;
    let registry = fixture
        .recorder
        .registry(&key)
        .expect("the gauge still holds its registry");
    assert_eq!(registry.len(), 3);
    assert!(
        registry.contains_key("slot-1"),
        "the rejected admission released nothing"
    );
    assert!(fixture.recorder.gauge_writes().is_empty());
}

#[tokio::test]
async fn capacity_releases_the_slot_after_a_normal_exit() {
    let fixture = Fixture::new(&Parts::new(
        entries([request_window("api_key", "key-1", 0)]),
        Vec::new(),
        None,
    ));
    let auth = limited_auth();
    let key = parallel_key("api_key", "key-1");

    let guard = fixture
        .hook
        .request_capacity(&auth, "gpt4", None)
        .await
        .expect("the capacity admission passes");
    let held = fixture
        .recorder
        .registry(&key)
        .expect("the admission filled the gauge");
    assert_eq!(held.len(), 1);
    drop(guard);
    fixture.settle().await;

    let released = fixture
        .recorder
        .registry(&key)
        .expect("the gauge still holds a registry");
    assert!(released.is_empty());
}

#[tokio::test]
async fn capacity_releases_the_slot_after_the_caller_errors() {
    let fixture = Fixture::new(&Parts::new(
        entries([request_window("api_key", "key-1", 0)]),
        Vec::new(),
        None,
    ));
    let auth = limited_auth();

    let outcome = failing_provider_call(&fixture, &auth).await;

    assert_eq!(outcome, Err("the provider call failed"));
    fixture.settle().await;
    let registry = fixture
        .recorder
        .registry(&parallel_key("api_key", "key-1"))
        .expect("the gauge still holds a registry");
    assert!(registry.is_empty());
}

async fn failing_provider_call(
    fixture: &Fixture,
    auth: &UserAPIKeyAuth,
) -> Result<(), &'static str> {
    fixture
        .hook
        .request_capacity(auth, "gpt4", None)
        .await
        .map_or(Ok(()), |_guard| Err("the provider call failed"))
}

#[tokio::test]
async fn capacity_releases_the_slot_after_the_caller_is_cancelled() {
    let gate = Arc::new(Semaphore::new(0));
    let fixture = Fixture::new(&Parts::new(
        entries([request_window("api_key", "key-1", 0)]),
        Vec::new(),
        Some(Arc::clone(&gate)),
    ));
    let auth = limited_auth();

    let cancelled = timeout(
        Duration::from_millis(10),
        fixture.hook.request_capacity(&auth, "gpt4", None),
    )
    .await;

    assert!(
        cancelled.is_err(),
        "the caller was cancelled before the admission ended"
    );
    gate.add_permits(16);
    fixture.settle().await;
    let registry = fixture
        .recorder
        .registry(&parallel_key("api_key", "key-1"))
        .expect("the gauge still holds a registry");
    assert!(registry.is_empty());
}

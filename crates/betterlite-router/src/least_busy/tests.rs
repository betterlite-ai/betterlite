#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use rust_decimal::Decimal;
use tokio::runtime::{Builder, Runtime};

use crate::deployment_identity::spec::DeploymentSpec;
use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::router_observations::failure::AsyncFailureEvent;
use crate::router_observations::success::{RequestFacts, SuccessEvent};
use crate::routing_state_store::Settings as SharedSettings;
use crate::routing_state_store::error::StateStoreError;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{BreakerClock, RedisSettings, RoutingStateStore};
use crate::routing_value_store::{
    Clock, PinValue, RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreError,
    StoreRead, StoreSettings,
};

use super::error::{LeastBusyError, LeastBusyWarning, UnreadableCounts, UnwrittenCount};
use super::identity::{AttemptFacts, deployment_ref, request_count_key, request_count_keys};
use super::{
    CountsSource, IN_FLIGHT_COUNT_TTL_SECONDS, LeastBusy, SharedCounts, as_counts, counter_values,
    least_busy, local_counts,
};

const GROUP: &str = "gpt";
const MODEL: &str = "openai/gpt-4o-mini";

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

#[derive(Clone, Default)]
struct TestClock(Arc<Mutex<f64>>);

impl TestClock {
    fn at(&self, seconds: f64) {
        *self.0.lock().unwrap() = seconds;
    }

    fn clock(&self) -> Clock {
        let source = Arc::clone(&self.0);
        Arc::new(move || *source.lock().unwrap())
    }
}

fn deployment(id: &str) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(MODEL.to_owned()));
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(GROUP.to_owned(), params, info, JsonObject::new(), || {
        "generated".to_owned()
    })
    .unwrap()
}

fn unnamed_deployment() -> DeploymentSpec {
    DeploymentSpec {
        model_name: GROUP.to_owned(),
        litellm_params: JsonObject::new(),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

fn facts<'facts>(
    group: Option<&'facts str>,
    deployment_id: Option<&'facts str>,
    call_type: Option<&'facts str>,
) -> AttemptFacts<'facts> {
    AttemptFacts {
        model_group: group,
        deployment_id,
        call_type,
    }
}

fn local_store(clock: &TestClock) -> RoutingValueStore {
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: Some(200),
            default_ttl_seconds: None,
            max_size_per_item: Some(1024),
            loaded_max_size_per_item_kib: 1024,
        },
        clock.clock(),
    )
    .unwrap()
}

fn shared_settings(port: u16, failure_threshold: u32) -> SharedSettings {
    SharedSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(port)),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 0.25,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold,
            recovery_timeout: Duration::from_secs(60),
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(1),
        },
    }
}

fn shared_store(server: &FakeRedis, failure_threshold: u32) -> RoutingStateStore {
    let settings = shared_settings(server.port(), failure_threshold);
    let clock: BreakerClock = Arc::new(|| Duration::ZERO);
    RoutingStateStore::new(&settings, clock).unwrap()
}

fn service(local: RoutingValueStore, shared: Option<RoutingStateStore>) -> Arc<LeastBusy> {
    Arc::new(LeastBusy { local, shared })
}

fn seed_local(store: &RoutingValueStore, deployment_id: &str, count: i64) {
    let key = request_count_key(GROUP, deployment_id).unwrap();
    store
        .increment(&key, &RouteIncrement::Counter(count), Some(600.0))
        .unwrap();
}

async fn seed_shared(store: &RoutingStateStore, deployment_id: &str, count: i64) {
    let key = request_count_key(GROUP, deployment_id).unwrap();
    store
        .increment(
            &key,
            &RouteIncrement::Counter(count),
            Some(IN_FLIGHT_COUNT_TTL_SECONDS),
        )
        .await
        .unwrap();
}

fn group_key(deployment_id: &str) -> RouteKey {
    request_count_key(GROUP, deployment_id).unwrap()
}

fn unreadable(warning: Option<LeastBusyWarning>) -> (String, UnreadableCounts) {
    match warning {
        Some(LeastBusyWarning::Unreadable { model_group, cause }) => (model_group, cause),
        None => panic!("the selection carried no warning"),
        Some(other) => panic!("unexpected warning {other:?}"),
    }
}

fn unwritten(warning: Option<LeastBusyWarning>) -> UnwrittenCount {
    match warning {
        Some(LeastBusyWarning::Unwritable { cause, .. }) => cause,
        None => panic!("the call carried no warning"),
        Some(other) => panic!("unexpected warning {other:?}"),
    }
}

async fn assert_counts(
    service: &LeastBusy,
    deployments: &[DeploymentSpec],
    shared: &[Option<i64>],
    local: &[Option<i64>],
) {
    let keys = request_count_keys(GROUP, deployments).unwrap();
    assert_eq!(service.local_count_values(&keys).unwrap(), local);
    match service.shared_counts(&keys).await {
        SharedCounts::Values(values) => assert_eq!(values, shared),
        SharedCounts::Absent => panic!("the shared tier is absent"),
        SharedCounts::Unreadable(cause) => panic!("the shared tier is unreadable: {cause:?}"),
    }
}

async fn shared_counts_win() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let local = local_store(&TestClock::default());
    seed_shared(&shared, "a", 2).await;
    seed_shared(&shared, "b", 1).await;
    seed_shared(&shared, "c", 1).await;
    seed_local(&local, "a", 0);
    seed_local(&local, "b", 5);
    seed_local(&local, "c", 5);
    let service = service(local, Some(shared));
    let deployments = [deployment("a"), deployment("b"), deployment("c")];
    let selection = service.select(GROUP, &deployments).await.unwrap();
    assert_eq!(selection.index, Some(1));
    assert_eq!(selection.source, CountsSource::Shared);
    assert!(selection.warning.is_none());
    assert_counts(
        &service,
        &deployments,
        &[Some(2), Some(1), Some(1)],
        &[Some(0), Some(5), Some(5)],
    )
    .await;
}

async fn local_without_shared() {
    let local = local_store(&TestClock::default());
    seed_local(&local, "a", 3);
    seed_local(&local, "b", 1);
    let service = service(local, None);
    let deployments = [deployment("a"), deployment("b")];
    let selection = service.select(GROUP, &deployments).await.unwrap();
    assert_eq!(selection.index, Some(1));
    assert_eq!(selection.source, CountsSource::Local);
    assert!(selection.warning.is_none());
}

async fn unavailable_falls_back() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let local = local_store(&TestClock::default());
    seed_local(&local, "a", 0);
    seed_local(&local, "b", 2);
    let service = service(local, Some(shared));
    server.set_down(true);
    let deployments = [deployment("a"), deployment("b")];
    let selection = service.select(GROUP, &deployments).await.unwrap();
    assert_eq!(selection.index, Some(0));
    assert_eq!(selection.source, CountsSource::Fallback);
    let (model_group, cause) = unreadable(selection.warning);
    assert_eq!(model_group, GROUP);
    assert!(matches!(
        cause,
        UnreadableCounts::Store {
            operation: "async_batch_get_cache"
        }
    ));
}

async fn circuit_open_falls_back() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 1);
    let service = service(local_store(&TestClock::default()), Some(shared));
    server.set_down(true);
    let deployments = [deployment("a")];
    let first = service.select(GROUP, &deployments).await.unwrap();
    assert_eq!(first.source, CountsSource::Fallback);
    assert!(matches!(
        unreadable(first.warning).1,
        UnreadableCounts::Store { .. }
    ));
    let second = service.select(GROUP, &deployments).await.unwrap();
    assert_eq!(second.index, Some(0));
    assert!(matches!(
        unreadable(second.warning).1,
        UnreadableCounts::Refused(StateStoreError::CircuitOpen { .. })
    ));
}

async fn spend_shared_falls_back() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let key = group_key("a");
    shared
        .increment(
            &key,
            &RouteIncrement::Spend(Decimal::ONE),
            Some(IN_FLIGHT_COUNT_TTL_SECONDS),
        )
        .await
        .unwrap();
    let service = service(local_store(&TestClock::default()), Some(shared));
    let selection = service.select(GROUP, &[deployment("a")]).await.unwrap();
    assert_eq!(selection.source, CountsSource::Fallback);
    match unreadable(selection.warning).1 {
        UnreadableCounts::NotACount { key } => assert_eq!(key, "gpt_request_count:a"),
        other => panic!("unexpected cause {other:?}"),
    }
}

async fn increment_and_release() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let service = service(local_store(&TestClock::default()), Some(shared));
    let deployments = [deployment("a")];
    let started = service
        .pre_call(&facts(Some(GROUP), Some("a"), None))
        .await
        .unwrap();
    assert!(started.warning.is_none());
    assert_counts(&service, &deployments, &[Some(1)], &[Some(1)]).await;
    assert!(started.call.unwrap().release().await.is_none());
    assert_counts(&service, &deployments, &[Some(0)], &[Some(0)]).await;
}

async fn clamp_after_reset() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let service = service(local_store(&TestClock::default()), Some(shared.clone()));
    let started = service
        .pre_call(&facts(Some(GROUP), Some("a"), None))
        .await
        .unwrap();
    let key = group_key("a");
    service
        .local
        .set(
            &key,
            &RouteValue::Counter(0),
            Some(IN_FLIGHT_COUNT_TTL_SECONDS),
        )
        .unwrap();
    let reset = shared
        .compare_and_set(
            &key,
            &StoreRead::Value(RouteValue::Counter(1)),
            &RouteValue::Counter(0),
            Some(IN_FLIGHT_COUNT_TTL_SECONDS),
        )
        .await
        .unwrap();
    assert!(reset.value.swapped);
    assert!(started.call.unwrap().release().await.is_none());
    assert_eq!(
        service.local_count_values(&[key.clone()]).unwrap(),
        vec![Some(0)]
    );
    match service.shared_counts(&[key]).await {
        SharedCounts::Values(values) => assert_eq!(values, vec![Some(0)]),
        other => panic!("unexpected shared counts {other:?}"),
    }
}

async fn drop_releases_local() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let service = service(local_store(&TestClock::default()), Some(shared));
    let deployments = [deployment("a")];
    let started = service
        .pre_call(&facts(Some(GROUP), Some("a"), None))
        .await
        .unwrap();
    drop(started.call.unwrap());
    assert_counts(&service, &deployments, &[Some(1)], &[Some(0)]).await;
}

async fn down_shared_warns() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let service = service(local_store(&TestClock::default()), Some(shared));
    server.set_down(true);
    let started = service
        .pre_call(&facts(Some(GROUP), Some("a"), None))
        .await
        .unwrap();
    assert!(matches!(
        unwritten(started.warning),
        UnwrittenCount::Shared(_)
    ));
    let deployments = [deployment("a")];
    let keys = request_count_keys(GROUP, &deployments).unwrap();
    assert_eq!(service.local_count_values(&keys).unwrap(), vec![Some(1)]);
    assert!(matches!(
        service.shared_counts(&keys).await,
        SharedCounts::Unreadable(UnreadableCounts::Store { .. })
    ));
}

async fn batch_retrieve_skips() {
    let service = service(local_store(&TestClock::default()), None);
    let skipped = service
        .pre_call(&facts(Some(GROUP), Some("a"), Some("retrieve_batch")))
        .await
        .unwrap();
    assert!(skipped.call.is_none());
    assert!(skipped.warning.is_none());
    let also_skipped = service
        .pre_call(&facts(Some(GROUP), Some("a"), Some("aretrieve_batch")))
        .await
        .unwrap();
    assert!(also_skipped.call.is_none());
    assert_eq!(
        service.local_count_values(&[group_key("a")]).unwrap(),
        vec![None]
    );
}

async fn missing_halves_skip() {
    let service = service(local_store(&TestClock::default()), None);
    let no_group = service
        .pre_call(&facts(None, Some("a"), None))
        .await
        .unwrap();
    assert!(no_group.call.is_none());
    assert!(no_group.warning.is_none());
    let no_id = service
        .pre_call(&facts(Some(GROUP), None, None))
        .await
        .unwrap();
    assert!(no_id.call.is_none());
    assert_eq!(
        service.local_count_values(&[group_key("a")]).unwrap(),
        vec![None]
    );
}

fn first_minimum() {
    assert_eq!(least_busy(3, &[2, 1, 1]), Some(1));
    assert_eq!(least_busy(3, &[1, 2, 3]), Some(0));
    assert_eq!(least_busy(3, &[0, -1, 0]), Some(1));
    assert_eq!(least_busy(3, &[3, 3, 3]), Some(0));
    assert_eq!(least_busy(0, &[]), None);
    assert_eq!(least_busy(2, &[5, 4, 0]), Some(1));
}

fn missing_counts_are_zero() {
    assert_eq!(as_counts(&[None, Some(4), Some(0)]), vec![0, 4, 0]);
    assert_eq!(as_counts(&[]), Vec::<i64>::new());
}

fn local_counts_length_rule() {
    assert_eq!(local_counts(None, 2), vec![0, 0]);
    assert_eq!(local_counts(Some(&[Some(1)]), 2), vec![0, 0]);
    assert_eq!(local_counts(Some(&[Some(1), None]), 2), vec![1, 0]);
}

fn counter_value_rule() {
    let first = group_key("a");
    let second = group_key("b");
    let pin = RouteValue::Pin(PinValue {
        model_id: "x".to_owned(),
    });
    assert_eq!(
        counter_values(
            &[first.clone(), second.clone()],
            vec![StoreRead::Missing, StoreRead::Value(RouteValue::Counter(4))]
        )
        .unwrap(),
        vec![None, Some(4)]
    );
    assert_eq!(
        counter_values(
            &[first.clone(), second.clone()],
            vec![
                StoreRead::Value(pin),
                StoreRead::Value(RouteValue::Counter(4))
            ]
        )
        .unwrap_err(),
        first
    );
    assert_eq!(
        counter_values(
            &[second.clone()],
            vec![StoreRead::Value(RouteValue::Spend(Decimal::ONE))]
        )
        .unwrap_err(),
        second
    );
}

fn attempt_facts_from_events() {
    let request = RequestFacts {
        model_group: Some(GROUP),
        deployment_name: "a",
        deployment_id: Some("a"),
    };
    let success = SuccessEvent {
        call_type: Some("acompletion"),
        total_tokens: None,
        request,
        deployment: None,
    };
    assert_eq!(
        AttemptFacts::from_success(&success),
        facts(Some(GROUP), Some("a"), Some("acompletion"))
    );
    let failure = AsyncFailureEvent {
        call_type: Some("aretrieve_batch"),
        model_group: Some(GROUP),
        deployment_name: "a",
        deployment_id: None,
    };
    assert_eq!(
        AttemptFacts::from_failure(&failure),
        facts(Some(GROUP), None, Some("aretrieve_batch"))
    );
    assert!(deployment_ref(&AttemptFacts::from_success(&success)).is_some());
    assert!(deployment_ref(&AttemptFacts::from_failure(&failure)).is_none());
}

fn local_values_read() {
    let service = LeastBusy {
        local: local_store(&TestClock::default()),
        shared: None,
    };
    let key = group_key("a");
    assert_eq!(
        service.local_count_values(&[key.clone()]).unwrap(),
        vec![None]
    );
    seed_local(&service.local, "a", 2);
    assert_eq!(service.local_count_values(&[key]).unwrap(), vec![Some(2)]);
}

async fn shared_absent() {
    let service = LeastBusy {
        local: local_store(&TestClock::default()),
        shared: None,
    };
    assert!(matches!(
        service.shared_counts(&[group_key("a")]).await,
        SharedCounts::Absent
    ));
}

fn ttl_expiry() {
    assert_eq!(IN_FLIGHT_COUNT_TTL_SECONDS.to_bits(), 3600.0_f64.to_bits());
    let clock = TestClock::default();
    let service = LeastBusy {
        local: local_store(&clock),
        shared: None,
    };
    seed_local(&service.local, "a", 1);
    let key = group_key("a");
    clock.at(601.0);
    assert_eq!(
        service.local_count_values(&[key.clone()]).unwrap(),
        vec![Some(1)]
    );
    clock.at(3601.0);
    assert_eq!(service.local_count_values(&[key]).unwrap(), vec![None]);
}

async fn behavior(name: &str) -> bool {
    match name {
        "shared_counts_win" => shared_counts_win().await,
        "local_without_shared" => local_without_shared().await,
        "unavailable_falls_back" => unavailable_falls_back().await,
        "circuit_open_falls_back" => circuit_open_falls_back().await,
        "spend_shared_falls_back" => spend_shared_falls_back().await,
        "increment_and_release" => increment_and_release().await,
        "clamp_after_reset" => clamp_after_reset().await,
        "drop_releases_local" => drop_releases_local().await,
        "down_shared_warns" => down_shared_warns().await,
        "batch_retrieve_skips" => batch_retrieve_skips().await,
        "missing_halves_skip" => missing_halves_skip().await,
        "first_minimum" => first_minimum(),
        "missing_counts_are_zero" => missing_counts_are_zero(),
        "local_counts_length_rule" => local_counts_length_rule(),
        "counter_value_rule" => counter_value_rule(),
        "attempt_facts_from_events" => attempt_facts_from_events(),
        "local_values_read" => local_values_read(),
        "shared_absent" => shared_absent().await,
        "ttl_expiry" => ttl_expiry(),
        _ => return false,
    }
    true
}

fn key_lengths() {
    let allowed = "g".repeat(1008);
    let key = request_count_key(&allowed, "d").unwrap();
    assert_eq!(key.as_ref().len(), 1024);
    let over = request_count_key(&"g".repeat(1009), "d").unwrap_err();
    assert!(matches!(
        over,
        LeastBusyError::Key {
            source: StoreError::KeyTooLong {
                bytes: 1025,
                limit: 1024
            }
        }
    ));
}

async fn missing_deployment_id_index() {
    let service = LeastBusy {
        local: local_store(&TestClock::default()),
        shared: None,
    };
    let error = service
        .select(GROUP, &[deployment("a"), unnamed_deployment()])
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        LeastBusyError::MissingDeploymentId { index: 1 }
    ));
    let keys = request_count_keys(GROUP, &[unnamed_deployment()]).unwrap_err();
    assert!(matches!(
        keys,
        LeastBusyError::MissingDeploymentId { index: 0 }
    ));
}

async fn local_spend_is_error() {
    let local = local_store(&TestClock::default());
    let key = group_key("a");
    local
        .set(&key, &RouteValue::Spend(Decimal::ONE), Some(3600.0))
        .unwrap();
    let service = LeastBusy {
        local,
        shared: None,
    };
    let error = service.select(GROUP, &[deployment("a")]).await.unwrap_err();
    assert!(matches!(
        error,
        LeastBusyError::NotACount { key } if key == "gpt_request_count:a"
    ));
}

async fn empty_pool() {
    let plain = LeastBusy {
        local: local_store(&TestClock::default()),
        shared: None,
    };
    let local_only = plain.select(GROUP, &[]).await.unwrap();
    assert_eq!(local_only.index, None);
    assert_eq!(local_only.source, CountsSource::Local);
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let with_shared = service(local_store(&TestClock::default()), Some(shared));
    let empty = with_shared.select(GROUP, &[]).await.unwrap();
    assert_eq!(empty.index, None);
    assert_eq!(empty.source, CountsSource::Shared);
}

fn selection_bounds() {
    assert_eq!(least_busy(1, &[i64::MAX]), Some(0));
    assert_eq!(least_busy(1, &[]), None);
    assert_eq!(least_busy(0, &[0, 0]), None);
}

async fn shared_missing_is_zero() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    seed_shared(&shared, "a", 2).await;
    let service = service(local_store(&TestClock::default()), Some(shared));
    let deployments = [deployment("a"), deployment("b")];
    let selection = service.select(GROUP, &deployments).await.unwrap();
    assert_eq!(selection.index, Some(1));
    let keys = request_count_keys(GROUP, &deployments).unwrap();
    match service.shared_counts(&keys).await {
        SharedCounts::Values(values) => assert_eq!(values, vec![Some(2), None]),
        other => panic!("unexpected shared counts {other:?}"),
    }
}

async fn cancel_drop() {
    let server = FakeRedis::try_start().await.unwrap();
    let shared = shared_store(&server, 100);
    let service = service(local_store(&TestClock::default()), Some(shared));
    let deployments = [deployment("a")];
    let started = service
        .pre_call(&facts(Some(GROUP), Some("a"), None))
        .await
        .unwrap();
    let call = started.call.unwrap();
    service
        .local
        .set(
            &group_key("a"),
            &RouteValue::Counter(0),
            Some(IN_FLIGHT_COUNT_TTL_SECONDS),
        )
        .unwrap();
    drop(call);
    assert_counts(&service, &deployments, &[Some(1)], &[Some(0)]).await;
}

async fn pre_call_over_limit() {
    let service = service(local_store(&TestClock::default()), None);
    let group = "g".repeat(1009);
    let error = match service
        .pre_call(&facts(Some(&group), Some("d"), None))
        .await
    {
        Err(error) => error,
        Ok(_started) => panic!("the key is past the limit"),
    };
    assert!(matches!(error, LeastBusyError::Key { .. }));
    assert_eq!(service.local_count_values(&[]).unwrap(), Vec::<i64>::new());
}

async fn boundary(name: &str) -> bool {
    match name {
        "key_lengths" => key_lengths(),
        "missing_deployment_id_index" => missing_deployment_id_index().await,
        "local_spend_is_error" => local_spend_is_error().await,
        "empty_pool" => empty_pool().await,
        "selection_bounds" => selection_bounds(),
        "shared_missing_is_zero" => shared_missing_is_zero().await,
        "cancel_drop" => cancel_drop().await,
        "pre_call_over_limit" => pre_call_over_limit().await,
        _ => return false,
    }
    true
}

#[rstest]
#[case::shared_counts_win("shared_counts_win")]
#[case::local_without_shared("local_without_shared")]
#[case::unavailable_falls_back("unavailable_falls_back")]
#[case::circuit_open_falls_back("circuit_open_falls_back")]
#[case::spend_shared_falls_back("spend_shared_falls_back")]
#[case::increment_and_release("increment_and_release")]
#[case::clamp_after_reset("clamp_after_reset")]
#[case::drop_releases_local("drop_releases_local")]
#[case::down_shared_warns("down_shared_warns")]
#[case::batch_retrieve_skips("batch_retrieve_skips")]
#[case::missing_halves_skip("missing_halves_skip")]
#[case::first_minimum("first_minimum")]
#[case::missing_counts_are_zero("missing_counts_are_zero")]
#[case::local_counts_length_rule("local_counts_length_rule")]
#[case::counter_value_rule("counter_value_rule")]
#[case::attempt_facts_from_events("attempt_facts_from_events")]
#[case::local_values_read("local_values_read")]
#[case::shared_absent("shared_absent")]
#[case::ttl_expiry("ttl_expiry")]
fn least_busy_behaviors(#[case] name: &str) {
    assert!(runtime().block_on(behavior(name)), "unknown case {name}");
}

#[rstest]
#[case::key_lengths("key_lengths")]
#[case::missing_deployment_id_index("missing_deployment_id_index")]
#[case::local_spend_is_error("local_spend_is_error")]
#[case::empty_pool("empty_pool")]
#[case::selection_bounds("selection_bounds")]
#[case::shared_missing_is_zero("shared_missing_is_zero")]
#[case::cancel_drop("cancel_drop")]
#[case::pre_call_over_limit("pre_call_over_limit")]
fn least_busy_boundaries(#[case] name: &str) {
    assert!(runtime().block_on(boundary(name)), "unknown case {name}");
}

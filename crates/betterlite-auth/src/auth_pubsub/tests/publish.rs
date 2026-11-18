use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::yield_now;
use tokio::time::timeout;

use crate::auth_pubsub::message::Invalidation;
use crate::auth_pubsub::publish::{
    MAX_IN_FLIGHT_PUBLISHES, MAX_PENDING_PUBLISHES, PublishTasks, evict_and_broadcast,
    publish_invalidation,
};

use super::support::{
    CapturedEvent, Entry, Probe, PublishMode, ScriptedEndpoint, ScriptedEviction, capture, runtime,
};

fn invalidation(cache_key: &str) -> Invalidation {
    Invalidation {
        cache_key: cache_key.to_owned(),
        new_value: None,
        ttl: None,
    }
}

fn endpoint(probe: &Arc<Probe>, mode: PublishMode) -> Arc<ScriptedEndpoint> {
    Arc::new(ScriptedEndpoint::new(Arc::clone(probe), Vec::new(), mode))
}

fn warning(subject: &str) -> CapturedEvent {
    CapturedEvent {
        level: "warn".to_owned(),
        subject: Some(subject.to_owned()),
        backoff_seconds: None,
    }
}

fn tag(index: usize) -> String {
    format!("tag:{index}")
}

async fn drain_blocked(release: &Notify, probe: &Probe, expected: usize) {
    for _attempt in 0..1000 {
        release.notify_waiters();
        yield_now().await;
        if probe.publishes() == expected {
            return;
        }
    }
    panic!("the blocked publishes never finished");
}

async fn publish_range(tasks: &PublishTasks, endpoint: &Arc<ScriptedEndpoint>, count: usize) {
    for index in 0..count {
        publish_invalidation(tasks, Some(endpoint), &invalidation(&tag(index))).await;
    }
}

async fn wait_for_publishes(probe: &Probe, expected: usize) {
    for _attempt in 0..1000 {
        if probe.publishes() == expected {
            return;
        }
        yield_now().await;
    }
    panic!("the publishes never finished");
}

#[test]
fn local_eviction_before_publish() {
    let runtime = runtime();
    let probe = Probe::new();
    let endpoint = endpoint(&probe, PublishMode::Accept);
    let eviction = ScriptedEviction::new(Arc::clone(&probe), Vec::new());
    let tasks = PublishTasks::default();
    runtime.block_on(async {
        let keys = ["tag:one".to_owned(), "tag:two".to_owned()];
        evict_and_broadcast(&keys, &eviction, &tasks, Some(&endpoint)).await;
    });
    assert_eq!(
        probe.events(),
        [
            Entry::Evict("tag:one".to_owned()),
            Entry::PublishStarted,
            Entry::Publish("{\"cache_key\":\"tag:one\"}".to_owned()),
            Entry::Evict("tag:two".to_owned()),
            Entry::PublishStarted,
            Entry::Publish("{\"cache_key\":\"tag:two\"}".to_owned()),
        ]
    );
}

#[test]
fn redis_outage_warning() {
    let runtime = runtime();
    let probe = Probe::new();
    let endpoint = endpoint(&probe, PublishMode::Fail);
    let eviction = ScriptedEviction::new(Arc::clone(&probe), Vec::new());
    let tasks = PublishTasks::default();
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            evict_and_broadcast(
                &["tag:outage".to_owned()],
                &eviction,
                &tasks,
                Some(&endpoint),
            )
            .await;
        });
    });
    assert_eq!(
        probe.events(),
        [
            Entry::Evict("tag:outage".to_owned()),
            Entry::PublishStarted,
            Entry::PublishFailed,
        ]
    );
    assert_eq!(captured, [warning("tag:outage")]);
}

#[test]
fn no_redis_noop() {
    let runtime = runtime();
    let probe = Probe::new();
    let eviction = ScriptedEviction::new(Arc::clone(&probe), Vec::new());
    let tasks = PublishTasks::default();
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            let absent = None::<&Arc<ScriptedEndpoint>>;
            evict_and_broadcast(&["tag:local".to_owned()], &eviction, &tasks, absent).await;
            publish_invalidation(&tasks, absent, &invalidation("tag:local")).await;
        });
    });
    assert_eq!(probe.events(), [Entry::Evict("tag:local".to_owned())]);
    assert!(captured.is_empty());
}

#[test]
fn completed_publish_slots_reused() {
    let runtime = runtime();
    let probe = Probe::new();
    let endpoint = endpoint(&probe, PublishMode::Accept);
    let tasks = PublishTasks::default();
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            publish_range(&tasks, &endpoint, MAX_PENDING_PUBLISHES.saturating_add(1)).await;
            wait_for_publishes(&probe, MAX_PENDING_PUBLISHES.saturating_add(1)).await;
        });
    });
    assert_eq!(
        probe.publishes(),
        MAX_PENDING_PUBLISHES.saturating_add(1),
        "finished publishes are pruned, so the pending limit never fills"
    );
    assert!(captured.is_empty());
}

#[test]
fn pending_1024_warns_drops() {
    let runtime = runtime();
    let probe = Probe::new();
    let release = Arc::new(Notify::new());
    let endpoint = endpoint(&probe, PublishMode::Block(Arc::clone(&release)));
    let tasks = PublishTasks::default();
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            publish_range(&tasks, &endpoint, MAX_PENDING_PUBLISHES).await;
            assert_eq!(probe.started(), MAX_IN_FLIGHT_PUBLISHES);
            let dropped = invalidation("tag:dropped");
            publish_invalidation(&tasks, Some(&endpoint), &dropped).await;
            drain_blocked(&release, &probe, MAX_PENDING_PUBLISHES).await;
        });
    });
    assert_eq!(probe.publishes(), MAX_PENDING_PUBLISHES);
    assert_eq!(probe.started(), MAX_PENDING_PUBLISHES);
    assert_eq!(captured, [warning("tag:dropped")]);
    let published_dropped = probe
        .events()
        .iter()
        .any(|entry| entry == &Entry::Publish("{\"cache_key\":\"tag:dropped\"}".to_owned()));
    assert!(
        !published_dropped,
        "a dropped publication never reaches redis"
    );
}

#[test]
fn at_most_16_inflight() {
    let runtime = runtime();
    let probe = Probe::new();
    let release = Arc::new(Notify::new());
    let endpoint = endpoint(&probe, PublishMode::Block(Arc::clone(&release)));
    let tasks = PublishTasks::default();
    runtime.block_on(async {
        publish_range(&tasks, &endpoint, MAX_IN_FLIGHT_PUBLISHES.saturating_mul(2)).await;
        assert_eq!(probe.started(), MAX_IN_FLIGHT_PUBLISHES);
        assert_eq!(probe.peak_in_flight(), MAX_IN_FLIGHT_PUBLISHES);
        drain_blocked(&release, &probe, MAX_IN_FLIGHT_PUBLISHES.saturating_mul(2)).await;
    });
    assert_eq!(probe.publishes(), MAX_IN_FLIGHT_PUBLISHES.saturating_mul(2));
    assert!(probe.peak_in_flight() <= MAX_IN_FLIGHT_PUBLISHES);
}

#[test]
fn publish_does_not_wait() {
    let runtime = runtime();
    let probe = Probe::new();
    let release = Arc::new(Notify::new());
    let endpoint = endpoint(&probe, PublishMode::Block(Arc::clone(&release)));
    let tasks = PublishTasks::default();
    runtime.block_on(async {
        let message = invalidation("tag:slow");
        let started = timeout(
            Duration::from_millis(50),
            publish_invalidation(&tasks, Some(&endpoint), &message),
        )
        .await;
        assert!(
            started.is_ok(),
            "the caller returns without the publish reply"
        );
        assert_eq!(probe.started(), 1);
        assert_eq!(probe.publishes(), 0);
        drain_blocked(&release, &probe, 1).await;
    });
    assert_eq!(probe.publishes(), 1);
}

#[test]
fn eviction_failure_still_publishes() {
    let runtime = runtime();
    let probe = Probe::new();
    let endpoint = endpoint(&probe, PublishMode::Accept);
    let eviction = ScriptedEviction::new(Arc::clone(&probe), vec!["tag:broken".to_owned()]);
    let tasks = PublishTasks::default();
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            let keys = ["tag:broken".to_owned()];
            evict_and_broadcast(&keys, &eviction, &tasks, Some(&endpoint)).await;
        });
    });
    assert_eq!(
        probe.events(),
        [
            Entry::Evict("tag:broken".to_owned()),
            Entry::EvictFailed,
            Entry::PublishStarted,
            Entry::Publish("{\"cache_key\":\"tag:broken\"}".to_owned()),
        ]
    );
    assert_eq!(captured, [warning("tag:broken")]);
}

#[test]
fn key_effect_order() {
    let runtime = runtime();
    let probe = Probe::new();
    let endpoint = endpoint(&probe, PublishMode::Accept);
    let eviction = ScriptedEviction::new(Arc::clone(&probe), vec!["tag:two".to_owned()]);
    let tasks = PublishTasks::default();
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            let keys = [
                "tag:one".to_owned(),
                "tag:two".to_owned(),
                "tag:three".to_owned(),
            ];
            evict_and_broadcast(&keys, &eviction, &tasks, Some(&endpoint)).await;
        });
    });
    assert_eq!(
        probe.events(),
        [
            Entry::Evict("tag:one".to_owned()),
            Entry::PublishStarted,
            Entry::Publish("{\"cache_key\":\"tag:one\"}".to_owned()),
            Entry::Evict("tag:two".to_owned()),
            Entry::EvictFailed,
            Entry::PublishStarted,
            Entry::Publish("{\"cache_key\":\"tag:two\"}".to_owned()),
            Entry::Evict("tag:three".to_owned()),
            Entry::PublishStarted,
            Entry::Publish("{\"cache_key\":\"tag:three\"}".to_owned()),
        ]
    );
    assert_eq!(captured, [warning("tag:two")]);
}

#[test]
fn shutdown_drains_publishes() {
    let runtime = runtime();
    let probe = Probe::new();
    let release = Arc::new(Notify::new());
    let endpoint = endpoint(&probe, PublishMode::Block(Arc::clone(&release)));
    let tasks = PublishTasks::default();
    runtime.block_on(async {
        publish_range(&tasks, &endpoint, 3).await;
        assert_eq!(probe.started(), 3);
        let drained = timeout(Duration::from_secs(5), tasks.shutdown()).await;
        assert!(
            drained.is_ok(),
            "shutdown returns once every pending publish is cancelled"
        );
        assert_eq!(
            probe.blocked_dropped(),
            3,
            "shutdown drops each blocked publish future"
        );
        assert_eq!(
            probe.publishes(),
            0,
            "a cancelled publish never reaches redis"
        );
    });
}

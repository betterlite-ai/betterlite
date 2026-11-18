use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_cache::dual_cache::DualCache;
use tokio::task::yield_now;
use tokio::time::timeout;

use crate::auth_pubsub::message::{Invalidation, encode};
use crate::auth_pubsub::subscriber::{
    BACKOFF_INITIAL, BACKOFF_MAX, CacheInvalidationSubscriber, POLL_TIMEOUT,
    Session as SubscriberSession,
};

use super::support::{
    CapturedEvent, CloseMode, Entry, FaultingMemory, Fixture, HEX_KEY, Probe, PublishMode,
    ScriptedClock, ScriptedEndpoint, Session as ScriptedSession, Step, capture, expiry, fixture,
    memory, runtime, store, stored,
};

type TestedSubscriber =
    CacheInvalidationSubscriber<ScriptedEndpoint, ScriptedClock, DualCache<FaultingMemory>>;

fn endpoint(probe: &Arc<Probe>, sessions: Vec<ScriptedSession>) -> Arc<ScriptedEndpoint> {
    Arc::new(ScriptedEndpoint::new(
        Arc::clone(probe),
        sessions,
        PublishMode::Accept,
    ))
}

fn subscriber(
    endpoint: Arc<ScriptedEndpoint>,
    probe: &Arc<Probe>,
    fixture: &Fixture,
    additional: Vec<Arc<FaultingMemory>>,
) -> Arc<TestedSubscriber> {
    Arc::new(CacheInvalidationSubscriber {
        endpoint,
        clock: ScriptedClock::new(Arc::clone(probe)),
        user_cache: Arc::clone(&fixture.cache),
        additional,
        session: Mutex::new(SubscriberSession::default()),
    })
}

fn invalidation(cache_key: &str, new_value: Option<f64>, ttl: Option<f64>) -> Invalidation {
    Invalidation {
        cache_key: cache_key.to_owned(),
        new_value,
        ttl,
    }
}

fn warning(subject: &str, backoff_seconds: Option<f64>) -> CapturedEvent {
    CapturedEvent {
        level: "warn".to_owned(),
        subject: Some(subject.to_owned()),
        backoff_seconds,
    }
}

async fn wait_until<F>(mut condition: F)
where
    F: FnMut() -> bool,
{
    for _attempt in 0..1000 {
        if condition() {
            return;
        }
        yield_now().await;
    }
    panic!("the subscriber never reached the expected state");
}

fn sleeps(probe: &Probe) -> Vec<Duration> {
    let mut slept = Vec::new();
    for entry in probe.events() {
        if let Entry::Sleep(duration) = entry {
            slept.push(duration);
        }
    }
    slept
}

#[test]
fn start_idempotent_stop_restart() {
    let runtime = runtime();
    let probe = Probe::new();
    let sessions = vec![
        ScriptedSession::Serving(vec![Step::Block]),
        ScriptedSession::Serving(vec![Step::Block]),
    ];
    let subscriber = subscriber(endpoint(&probe, sessions), &probe, &fixture(), Vec::new());
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.stop().await;
            assert!(
                probe.events().is_empty(),
                "stopping a subscriber that never started touches no redis"
            );
            subscriber.start();
            subscriber.start();
            subscriber.stop().await;
            subscriber.start();
            subscriber.stop().await;
        });
    });
    assert_eq!(
        probe.events(),
        [
            Entry::Subscribe,
            Entry::Poll(POLL_TIMEOUT),
            Entry::Close,
            Entry::Subscribe,
            Entry::Poll(POLL_TIMEOUT),
            Entry::Close,
        ]
    );
    assert!(captured.is_empty());
}

#[test]
fn shutdown_closes_pubsub() {
    let runtime = runtime();
    let probe = Probe::new();
    let sessions = vec![ScriptedSession::Serving(vec![Step::Block])];
    let subscriber = subscriber(endpoint(&probe, sessions), &probe, &fixture(), Vec::new());
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.start();
            wait_until(|| probe.count(&Entry::Subscribe) == 1).await;
            let stopped = timeout(Duration::from_secs(5), subscriber.stop()).await;
            assert!(
                stopped.is_ok(),
                "stop returns while the subscription is mid-poll"
            );
        });
    });
    assert_eq!(
        probe.events(),
        [Entry::Subscribe, Entry::Poll(POLL_TIMEOUT), Entry::Close]
    );
    assert!(captured.is_empty(), "a shutdown is not a redis failure");
}

#[test]
fn poll_timeout_and_empty() {
    let runtime = runtime();
    let probe = Probe::new();
    let key = "spend:team:1";
    let message = encode(&invalidation(key, Some(5.0_f64), Some(30.0_f64)));
    let sessions = vec![ScriptedSession::Serving(vec![
        Step::Empty,
        Step::Empty,
        Step::Message(message.into_bytes()),
        Step::Block,
    ])];
    let fixture = fixture();
    let additional = FaultingMemory::new(memory());
    let subscriber = subscriber(
        endpoint(&probe, sessions),
        &probe,
        &fixture,
        vec![Arc::clone(&additional)],
    );
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.start();
            wait_until(|| stored(&*additional, key) == Some(5.0_f64)).await;
            subscriber.stop().await;
        });
    });
    assert_eq!(
        probe.events(),
        [
            Entry::Subscribe,
            Entry::Poll(POLL_TIMEOUT),
            Entry::Poll(POLL_TIMEOUT),
            Entry::Poll(POLL_TIMEOUT),
            Entry::Message,
            Entry::Poll(POLL_TIMEOUT),
            Entry::Close,
        ],
        "each read uses a one second timeout and an empty poll reads again"
    );
    assert_eq!(expiry(&*additional, key), Some(1030.0_f64));
    assert!(captured.is_empty());
}

#[test]
fn reconnect_backoff_cap_reset() {
    let runtime = runtime();
    let probe = Probe::new();
    let sessions = vec![
        ScriptedSession::Failure,
        ScriptedSession::Failure,
        ScriptedSession::Failure,
        ScriptedSession::Failure,
        ScriptedSession::Failure,
        ScriptedSession::Failure,
        ScriptedSession::Serving(vec![Step::Fail]),
        ScriptedSession::Serving(vec![Step::Block]),
    ];
    let subscriber = subscriber(endpoint(&probe, sessions), &probe, &fixture(), Vec::new());
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.start();
            wait_until(|| probe.count(&Entry::Subscribe) == 8).await;
            subscriber.stop().await;
        });
    });
    let ten = Duration::from_secs(10);
    let twenty = Duration::from_secs(20);
    let forty = Duration::from_secs(40);
    assert_eq!(
        sleeps(&probe),
        [
            BACKOFF_INITIAL,
            ten,
            twenty,
            forty,
            BACKOFF_MAX,
            BACKOFF_MAX,
            BACKOFF_INITIAL,
        ],
        "the pause doubles to sixty seconds and a successful subscribe resets it"
    );
    let warned: Vec<Option<f64>> = captured
        .iter()
        .filter(|event| event.level == "warn")
        .map(|event| event.backoff_seconds)
        .collect();
    assert_eq!(
        warned,
        [
            Some(5.0_f64),
            Some(10.0_f64),
            Some(20.0_f64),
            Some(40.0_f64),
            Some(60.0_f64),
            Some(60.0_f64),
            Some(5.0_f64),
        ]
    );
}

#[test]
fn cancellation_no_backoff() {
    let runtime = runtime();
    let probe = Probe::new();
    let sessions = vec![ScriptedSession::Serving(vec![Step::Block])];
    let subscriber = subscriber(endpoint(&probe, sessions), &probe, &fixture(), Vec::new());
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.start();
            wait_until(|| probe.count(&Entry::Subscribe) == 1).await;
            subscriber.stop().await;
        });
    });
    assert!(
        sleeps(&probe).is_empty(),
        "a cancelled subscriber never backs off"
    );
    assert!(captured.is_empty(), "a cancelled subscriber never warns");
    assert_eq!(probe.count(&Entry::Close), 1);
}

#[test]
fn close_failure_debug() {
    let runtime = runtime();
    let probe = Probe::new();
    let sessions = vec![
        ScriptedSession::Serving(vec![Step::Fail]),
        ScriptedSession::Serving(vec![Step::Block]),
    ];
    let endpoint = Arc::new(
        ScriptedEndpoint::new(Arc::clone(&probe), sessions, PublishMode::Accept)
            .with_close(CloseMode::Fail),
    );
    let subscriber = subscriber(endpoint, &probe, &fixture(), Vec::new());
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.start();
            wait_until(|| probe.count(&Entry::Subscribe) == 2).await;
            subscriber.stop().await;
        });
    });
    assert_eq!(probe.count(&Entry::CloseFailed), 1);
    assert_eq!(
        captured,
        [
            CapturedEvent {
                level: "debug".to_owned(),
                subject: Some("the scripted close failed".to_owned()),
                backoff_seconds: None,
            },
            warning("the scripted consume failed", Some(5.0_f64)),
        ],
        "a close failure is debug-only and never replaces the consume failure"
    );
}

#[test]
fn set_additional_only() {
    let runtime = runtime();
    let probe = Probe::new();
    let key = "spend:team:1";
    let message = encode(&invalidation(key, Some(7.5_f64), Some(30.0_f64)));
    let sessions = vec![ScriptedSession::Serving(vec![
        Step::Message(message.into_bytes()),
        Step::Block,
    ])];
    let fixture = fixture();
    let extra_one = FaultingMemory::new(memory());
    let extra_two = FaultingMemory::new(memory());
    store(&*fixture.ordinary, key, 1.0_f64);
    store(&*extra_one, key, 1.0_f64);
    store(&*extra_two, key, 1.0_f64);
    let subscriber = subscriber(
        endpoint(&probe, sessions),
        &probe,
        &fixture,
        vec![Arc::clone(&extra_one), Arc::clone(&extra_two)],
    );
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.start();
            wait_until(|| stored(&*extra_two, key) == Some(7.5_f64)).await;
            subscriber.stop().await;
        });
    });
    assert_eq!(
        stored(&*fixture.ordinary, key),
        Some(1.0_f64),
        "a set never writes the primary user cache"
    );
    assert_eq!(stored(&*extra_one, key), Some(7.5_f64));
    assert_eq!(expiry(&*extra_one, key), Some(1030.0_f64));
    assert_eq!(stored(&*extra_two, key), Some(7.5_f64));
    assert_eq!(expiry(&*extra_two, key), Some(1030.0_f64));
    assert!(captured.is_empty());
    assert_eq!(probe.publishes(), 0, "a consumed set is not republished");
}

#[test]
fn delete_all_local_only() {
    let runtime = runtime();
    let probe = Probe::new();
    let message = encode(&invalidation(HEX_KEY, None, None));
    let sessions = vec![ScriptedSession::Serving(vec![
        Step::Message(message.into_bytes()),
        Step::Block,
    ])];
    let fixture = fixture();
    let extra = FaultingMemory::new(memory());
    store(&*fixture.key_objects, HEX_KEY, 2.0_f64);
    store(&*fixture.ordinary, HEX_KEY, 3.0_f64);
    store(&*extra, HEX_KEY, 4.0_f64);
    let subscriber = subscriber(
        endpoint(&probe, sessions),
        &probe,
        &fixture,
        vec![Arc::clone(&extra)],
    );
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.start();
            wait_until(|| stored(&*extra, HEX_KEY).is_none()).await;
            subscriber.stop().await;
        });
    });
    assert_eq!(
        stored(&*fixture.key_objects, HEX_KEY),
        None,
        "the primary cache is the partition in_memory_cache_for selects"
    );
    assert_eq!(
        stored(&*fixture.ordinary, HEX_KEY),
        Some(3.0_f64),
        "the other primary partition keeps its entry"
    );
    assert!(
        stored(&*extra, HEX_KEY).is_none(),
        "every additional cache is cleared"
    );
    assert!(captured.is_empty());
    assert_eq!(probe.publishes(), 0, "a consumed delete is not republished");
}

#[test]
fn nonfinite_counter_dropped() {
    let runtime = runtime();
    let probe = Probe::new();
    let key = "spend:team:nonfinite";
    let message = encode(&invalidation(key, Some(f64::INFINITY), Some(30.0_f64)));
    let sessions = vec![ScriptedSession::Serving(vec![
        Step::Message(message.into_bytes()),
        Step::Block,
    ])];
    let fixture = fixture();
    let extra = FaultingMemory::new(memory());
    store(&*fixture.ordinary, key, 2.0_f64);
    store(&*extra, key, 4.0_f64);
    let seeded_expiry = expiry(&*extra, key);
    let subscriber = subscriber(
        endpoint(&probe, sessions),
        &probe,
        &fixture,
        vec![Arc::clone(&extra)],
    );
    let ((), captured) = capture(|| {
        runtime.block_on(async {
            subscriber.start();
            wait_until(|| probe.count(&Entry::Poll(POLL_TIMEOUT)) == 2).await;
            subscriber.stop().await;
        });
    });
    assert_eq!(
        stored(&*extra, key),
        Some(4.0_f64),
        "a nonfinite counter never lands in an additional cache"
    );
    assert_eq!(
        expiry(&*extra, key),
        seeded_expiry,
        "a dropped counter leaves the stored expiry alone"
    );
    assert_eq!(stored(&*fixture.ordinary, key), Some(2.0_f64));
    assert_eq!(captured, [warning(key, None)]);
}

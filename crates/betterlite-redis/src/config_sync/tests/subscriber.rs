use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rstest::rstest;
use tokio::task::JoinSet;
use tokio::task::yield_now;
use tokio::time::timeout;

use super::support::{
    ClientStep, FakeClock, FixedJitter, Journal, Logs, MessageStep, ScriptedClient, TestSubscriber,
    recording_callback, session, subscription, test_subscriber,
};
use crate::config_sync::settings::ConfigSyncSettings;
use crate::config_sync::subscriber::ConfigSyncSubscriber;

fn quiet_flag() -> AtomicBool {
    AtomicBool::new(false)
}

fn empty_subscriber(
    clock: FakeClock,
) -> ConfigSyncSubscriber<ScriptedClient, FakeClock, &'static str> {
    test_subscriber(
        ScriptedClient::scripted(Vec::new(), &Journal::default()),
        clock,
        FixedJitter::fixed(Duration::ZERO),
        Vec::new(),
    )
}

async fn reconnect(
    subscriber: &TestSubscriber,
    backoff: &mut Duration,
    last_resync_at: &mut Option<Duration>,
) {
    subscriber
        .reconnect_once(backoff, last_resync_at, &quiet_flag())
        .await;
}

#[tokio::test]
async fn a_message_waits_debounce_plus_jitter_then_resyncs_once() {
    let journal = Journal::default();
    let client = ScriptedClient::scripted(
        vec![session(vec![
            MessageStep::Message("{}"),
            MessageStep::Empty,
            MessageStep::Fail,
        ])],
        &journal,
    );
    let jitter = FixedJitter::fixed(Duration::from_secs(2));
    let jitter_handle = jitter.clone();
    let subscriber = test_subscriber(
        client.clone(),
        FakeClock::default(),
        jitter,
        vec![recording_callback(&journal, "callback", Ok(()))],
    );
    let mut subscription = subscription(&client).await;
    let mut last_resync_at = None;
    let outcome = subscriber
        .consume(&mut subscription, &mut last_resync_at, &quiet_flag())
        .await;
    assert!(outcome.is_err());
    assert_eq!(
        journal.entries(),
        [
            "subscribe",
            "next:1.000",
            "next:0.000",
            "callback",
            "next:1.000"
        ]
    );
    assert_eq!(subscriber.clock.recorded_pauses(), [Duration::from_secs(3)]);
    assert_eq!(jitter_handle.recorded_limits(), [Duration::from_secs(5)]);
    assert_eq!(last_resync_at, Some(Duration::from_secs(3)));
}

#[tokio::test]
async fn pending_messages_are_drained_before_the_callbacks_run() {
    let journal = Journal::default();
    let client = ScriptedClient::scripted(
        vec![session(vec![
            MessageStep::Message("not json"),
            MessageStep::Message("{}"),
            MessageStep::Empty,
            MessageStep::Fail,
        ])],
        &journal,
    );
    let subscriber = test_subscriber(
        client.clone(),
        FakeClock::default(),
        FixedJitter::fixed(Duration::ZERO),
        vec![recording_callback(&journal, "callback", Ok(()))],
    );
    let mut subscription = subscription(&client).await;
    let mut last_resync_at = None;
    let outcome = subscriber
        .consume(&mut subscription, &mut last_resync_at, &quiet_flag())
        .await;
    assert!(outcome.is_err());
    assert_eq!(
        journal.entries(),
        [
            "subscribe",
            "next:1.000",
            "next:0.000",
            "next:0.000",
            "callback",
            "next:1.000",
        ]
    );
}

#[tokio::test]
async fn a_second_resync_waits_out_the_minimum_interval() {
    let journal = Journal::default();
    let client = ScriptedClient::scripted(
        vec![session(vec![
            MessageStep::Message("{}"),
            MessageStep::Empty,
            MessageStep::Message("{}"),
            MessageStep::Empty,
            MessageStep::Fail,
        ])],
        &journal,
    );
    let subscriber = test_subscriber(
        client.clone(),
        FakeClock::default(),
        FixedJitter::fixed(Duration::from_secs(2)),
        vec![recording_callback(&journal, "callback", Ok(()))],
    );
    let mut subscription = subscription(&client).await;
    let mut last_resync_at = None;
    let outcome = subscriber
        .consume(&mut subscription, &mut last_resync_at, &quiet_flag())
        .await;
    assert!(outcome.is_err());
    assert_eq!(
        subscriber.clock.recorded_pauses(),
        [
            Duration::from_secs(3),
            Duration::from_secs(3),
            Duration::from_secs(7),
        ]
    );
    assert_eq!(
        journal
            .entries()
            .iter()
            .filter(|entry| entry.as_str() == "callback")
            .count(),
        2
    );
    assert_eq!(last_resync_at, Some(Duration::from_secs(13)));
}

#[rstest]
#[case::no_previous_resync(None, None)]
#[case::just_inside_the_interval(
    Some(Duration::from_millis(9_999)),
    Some(Duration::from_millis(1))
)]
#[case::exactly_the_interval(Some(Duration::from_secs(10)), None)]
#[case::past_the_interval(Some(Duration::from_secs(11)), None)]
#[tokio::test]
async fn the_minimum_resync_interval_sleeps_only_the_remainder(
    #[case] elapsed: Option<Duration>,
    #[case] expected: Option<Duration>,
) {
    let clock = FakeClock::default();
    let last_resync_at = elapsed.map(|inside| {
        clock.advance(inside);
        Duration::ZERO
    });
    let subscriber = empty_subscriber(clock);
    subscriber
        .wait_for_min_resync_interval(last_resync_at)
        .await;
    let expected_pauses: Vec<Duration> = expected.into_iter().collect();
    assert_eq!(subscriber.clock.recorded_pauses(), expected_pauses);
}

#[tokio::test]
async fn a_failing_callback_warns_and_the_next_callback_still_runs() {
    let journal = Journal::default();
    let subscriber = test_subscriber(
        ScriptedClient::scripted(Vec::new(), &Journal::default()),
        FakeClock::default(),
        FixedJitter::fixed(Duration::ZERO),
        vec![
            recording_callback(&journal, "first", Err("boom")),
            recording_callback(&journal, "second", Ok(())),
        ],
    );
    let (logs, _guard) = Logs::capture();
    subscriber.run_resync_callbacks().await;
    assert_eq!(journal.entries(), ["first", "second"]);
    assert!(
        logs.lines()
            .iter()
            .any(|line| line.contains("config sync resync callback failed: boom")),
        "the failure is logged as a warning: {:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn subscribe_failures_double_the_backoff_up_to_sixty_seconds() {
    let journal = Journal::default();
    let client = ScriptedClient::scripted(Vec::new(), &journal);
    let subscriber = test_subscriber(
        client,
        FakeClock::default(),
        FixedJitter::fixed(Duration::ZERO),
        Vec::new(),
    );
    let mut backoff = subscriber.settings.backoff_initial;
    let mut last_resync_at = None;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    assert_eq!(
        subscriber.clock.recorded_pauses(),
        [
            Duration::from_secs(5),
            Duration::from_secs(10),
            Duration::from_secs(20),
            Duration::from_secs(40),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(60),
        ]
    );
}

#[tokio::test]
async fn a_successful_subscribe_resets_the_backoff_to_five_seconds() {
    let journal = Journal::default();
    let client = ScriptedClient::scripted(
        vec![
            ClientStep::Fail,
            ClientStep::Fail,
            session(vec![MessageStep::Fail]),
            session(vec![MessageStep::Fail]),
        ],
        &journal,
    );
    let subscriber = test_subscriber(
        client,
        FakeClock::default(),
        FixedJitter::fixed(Duration::ZERO),
        Vec::new(),
    );
    let mut backoff = subscriber.settings.backoff_initial;
    let mut last_resync_at = None;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    reconnect(&subscriber, &mut backoff, &mut last_resync_at).await;
    assert_eq!(
        subscriber.clock.recorded_pauses(),
        [
            Duration::from_secs(5),
            Duration::from_secs(10),
            Duration::from_secs(5),
            Duration::from_secs(5),
        ]
    );
}

#[tokio::test]
async fn the_subscriber_stops_when_the_server_shuts_down() {
    let journal = Journal::default();
    let client = ScriptedClient::scripted(vec![session(Vec::new())], &journal);
    let settings = ConfigSyncSettings {
        poll_timeout: Duration::from_millis(5),
        debounce: Duration::from_secs(1),
        jitter_max: Duration::from_secs(5),
        min_resync_interval: Duration::from_secs(10),
        backoff_initial: Duration::from_secs(5),
        backoff_max: Duration::from_secs(60),
    };
    let subscriber: Arc<ConfigSyncSubscriber<ScriptedClient, FakeClock, &'static str>> =
        Arc::new(ConfigSyncSubscriber {
            client: client.clone(),
            clock: FakeClock::default(),
            jitter: Arc::new(FixedJitter::fixed(Duration::ZERO)),
            callbacks: Vec::new(),
            settings,
        });
    let shutting_down = Arc::new(AtomicBool::new(false));
    let mut tasks = JoinSet::new();
    tasks.spawn({
        let task_subscriber = Arc::clone(&subscriber);
        let task_flag = Arc::clone(&shutting_down);
        async move { task_subscriber.run(&task_flag).await }
    });
    let joined = timeout(Duration::from_secs(2), async {
        while journal.first_index("subscribe").is_none() {
            yield_now().await;
        }
        shutting_down.store(true, Ordering::Release);
        tasks.join_next().await
    })
    .await
    .expect("the subscriber stops on shutdown");
    assert!(matches!(joined, Some(Ok(()))));
    assert!(journal.first_index("close").is_some());
}

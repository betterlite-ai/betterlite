use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::support::{
    ClientStep, FakeClock, FixedJitter, Journal, Logs, MessageStep, ScriptedClient, SlowClock,
    TestSubscriber, recording_callback, session, shutdown_channel, subscription, test_subscriber,
    wait_for_entry,
};
use crate::config_sync::settings::ConfigSyncSettings;
use crate::config_sync::subscriber::ConfigSyncSubscriber;

fn empty_subscriber(
    clock: FakeClock,
) -> ConfigSyncSubscriber<ScriptedClient, FakeClock, &'static str> {
    test_subscriber(
        ScriptedClient::scripted(Vec::new(), &Journal::default()),
        clock,
        FixedJitter::fixed(0),
        Vec::new(),
    )
}

struct ReconnectState {
    backoff: Duration,
    last_resync_at: Option<Duration>,
}

async fn reconnect(
    subscriber: &TestSubscriber,
    state: &mut ReconnectState,
    shutdown: &mut watch::Receiver<bool>,
) {
    let _stopped = subscriber
        .reconnect_once(&mut state.backoff, &mut state.last_resync_at, shutdown)
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
    let jitter = FixedJitter::fixed(2_000_000_000);
    let jitter_handle = jitter.clone();
    let subscriber = test_subscriber(
        client.clone(),
        FakeClock::recording(&journal),
        jitter,
        vec![recording_callback(&journal, "callback", Ok(()))],
    );
    let mut subscription = subscription(&client).await;
    let mut last_resync_at = None;
    let (_sender, mut shutdown) = shutdown_channel();
    let outcome = subscriber
        .consume(&mut subscription, &mut last_resync_at, &mut shutdown)
        .await;
    assert!(outcome.is_err());
    assert_eq!(
        journal.entries(),
        [
            "subscribe",
            "next:1.000",
            "pause:3.000",
            "next:0.000",
            "callback",
            "next:1.000"
        ]
    );
    assert_eq!(subscriber.clock.recorded_pauses(), [Duration::from_secs(3)]);
    assert_eq!(jitter_handle.recorded_limits(), [5_000_000_000]);
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
        FakeClock::recording(&journal),
        FixedJitter::fixed(0),
        vec![recording_callback(&journal, "callback", Ok(()))],
    );
    let mut subscription = subscription(&client).await;
    let mut last_resync_at = None;
    let (_sender, mut shutdown) = shutdown_channel();
    let outcome = subscriber
        .consume(&mut subscription, &mut last_resync_at, &mut shutdown)
        .await;
    assert!(outcome.is_err());
    assert_eq!(
        journal.entries(),
        [
            "subscribe",
            "next:1.000",
            "pause:1.000",
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
        FakeClock::recording(&journal),
        FixedJitter::fixed(2_000_000_000),
        vec![recording_callback(&journal, "callback", Ok(()))],
    );
    let mut subscription = subscription(&client).await;
    let mut last_resync_at = None;
    let (_sender, mut shutdown) = shutdown_channel();
    let outcome = subscriber
        .consume(&mut subscription, &mut last_resync_at, &mut shutdown)
        .await;
    assert!(outcome.is_err());
    assert_eq!(
        journal.entries(),
        [
            "subscribe",
            "next:1.000",
            "pause:3.000",
            "next:0.000",
            "callback",
            "next:1.000",
            "pause:3.000",
            "pause:7.000",
            "next:0.000",
            "callback",
            "next:1.000",
        ]
    );
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
    let (_sender, mut shutdown) = shutdown_channel();
    let stopped = subscriber
        .wait_for_min_resync_interval(last_resync_at, &mut shutdown)
        .await;
    assert!(!stopped);
    let expected_pauses: Vec<Duration> = expected.into_iter().collect();
    assert_eq!(subscriber.clock.recorded_pauses(), expected_pauses);
}

#[tokio::test]
async fn a_failing_callback_warns_and_the_next_callback_still_runs() {
    let journal = Journal::default();
    let subscriber = test_subscriber(
        ScriptedClient::scripted(Vec::new(), &Journal::default()),
        FakeClock::default(),
        FixedJitter::fixed(0),
        vec![
            recording_callback(&journal, "first", Err("boom")),
            recording_callback(&journal, "second", Ok(())),
        ],
    );
    let (logs, _guard) = Logs::capture();
    let (_sender, mut shutdown) = shutdown_channel();
    subscriber.run_resync_callbacks(&mut shutdown).await;
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
        FixedJitter::fixed(0),
        Vec::new(),
    );
    let mut state = ReconnectState {
        backoff: subscriber.settings.backoff_initial,
        last_resync_at: None,
    };
    let (_sender, mut shutdown) = shutdown_channel();
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
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
        FixedJitter::fixed(0),
        Vec::new(),
    );
    let mut state = ReconnectState {
        backoff: subscriber.settings.backoff_initial,
        last_resync_at: None,
    };
    let (_sender, mut shutdown) = shutdown_channel();
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    reconnect(&subscriber, &mut state, &mut shutdown).await;
    assert_eq!(
        subscriber.clock.recorded_pauses(),
        [
            Duration::from_secs(5),
            Duration::from_secs(10),
            Duration::from_secs(5),
            Duration::from_secs(5),
        ]
    );
    let closes = journal
        .entries()
        .iter()
        .filter(|entry| entry.as_str() == "close")
        .count();
    assert_eq!(closes, 2);
}

#[tokio::test]
async fn the_subscriber_stops_when_the_server_shuts_down() {
    let journal = Journal::default();
    let client = ScriptedClient::scripted(vec![session(Vec::new())], &journal);
    let settings = ConfigSyncSettings {
        poll_timeout: Duration::from_millis(5),
        debounce: Duration::from_secs(1),
        jitter_max_nanos: 5_000_000_000,
        min_resync_interval: Duration::from_secs(10),
        backoff_initial: Duration::from_secs(5),
        backoff_max: Duration::from_secs(60),
    };
    let subscriber: Arc<ConfigSyncSubscriber<ScriptedClient, FakeClock, &'static str>> =
        Arc::new(ConfigSyncSubscriber {
            client: client.clone(),
            clock: FakeClock::default(),
            jitter: Arc::new(FixedJitter::fixed(0)),
            callbacks: Vec::new(),
            settings,
        });
    let (sender, receiver) = shutdown_channel();
    let mut tasks = JoinSet::new();
    tasks.spawn({
        let task_subscriber = Arc::clone(&subscriber);
        async move { task_subscriber.run(receiver).await }
    });
    wait_for_entry(&journal, "subscribe").await;
    sender.send_replace(true);
    let joined = timeout(Duration::from_secs(2), tasks.join_next())
        .await
        .expect("the subscriber stops on shutdown");
    assert!(matches!(joined, Some(Ok(()))));
    assert!(journal.first_index("close").is_some());
}

#[tokio::test]
async fn shutdown_during_the_backoff_pause_returns_at_once() {
    let journal = Journal::default();
    let client = ScriptedClient::scripted(Vec::new(), &journal);
    let subscriber = Arc::new(test_subscriber(
        client,
        SlowClock::recording(&journal),
        FixedJitter::fixed(0),
        Vec::new(),
    ));
    let (sender, receiver) = shutdown_channel();
    let mut tasks = JoinSet::new();
    tasks.spawn({
        let task_subscriber = Arc::clone(&subscriber);
        async move { task_subscriber.run(receiver).await }
    });
    wait_for_entry(&journal, "pause-start:5.000").await;
    sender.send_replace(true);
    let joined = timeout(Duration::from_secs(2), tasks.join_next())
        .await
        .expect("the subscriber stops during the backoff pause");
    assert!(matches!(joined, Some(Ok(()))));
    assert!(journal.first_index("pause-done:5.000").is_none());
    assert!(journal.first_index("callback").is_none());
}

#[tokio::test]
async fn shutdown_during_the_debounce_pause_skips_the_callbacks() {
    let journal = Journal::default();
    let client =
        ScriptedClient::scripted(vec![session(vec![MessageStep::Message("{}")])], &journal);
    let subscriber = Arc::new(test_subscriber(
        client,
        SlowClock::recording(&journal),
        FixedJitter::fixed(2_000_000_000),
        vec![recording_callback(&journal, "callback", Ok(()))],
    ));
    let (sender, receiver) = shutdown_channel();
    let mut tasks = JoinSet::new();
    tasks.spawn({
        let task_subscriber = Arc::clone(&subscriber);
        async move { task_subscriber.run(receiver).await }
    });
    wait_for_entry(&journal, "pause-start:3.000").await;
    sender.send_replace(true);
    let joined = timeout(Duration::from_secs(2), tasks.join_next())
        .await
        .expect("the subscriber stops during the debounce pause");
    assert!(matches!(joined, Some(Ok(()))));
    assert!(journal.first_index("pause-done:3.000").is_none());
    assert!(journal.first_index("callback").is_none());
}

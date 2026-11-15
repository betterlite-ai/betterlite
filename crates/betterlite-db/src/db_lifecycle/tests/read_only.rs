use std::sync::{Arc, Mutex};
use std::time::Duration;

use rstest::rstest;

use super::support::{COOLDOWN, FakeReconnect, capture_logs, fake_clock, recreate_call};
use crate::auth_reconnect::clock::ReconnectClock;
use crate::db_lifecycle::read_only::{ReadOnlyWriter, RecreateState};

type TestedWriter = ReadOnlyWriter<FakeReconnect, super::support::FakeClock>;

fn writer(
    reconnect: &Arc<FakeReconnect>,
    cooldown: Duration,
    state: RecreateState,
) -> TestedWriter {
    ReadOnlyWriter {
        reconnect: Arc::clone(reconnect),
        cooldown,
        clock: fake_clock(),
        state: Mutex::new(state),
    }
}

#[tokio::test]
async fn first_read_only_recreate_warns_and_force_recreates() {
    let (logs, guard) = capture_logs();
    let reconnect = Arc::new(FakeReconnect::default());
    let read_only = writer(&reconnect, COOLDOWN, RecreateState::default());
    let recreated = read_only
        .recreate("postgres_read_only_transaction", None)
        .await;
    drop(guard);
    assert!(recreated);
    assert_eq!(
        reconnect.called(),
        [recreate_call("postgres_read_only_transaction", None)]
    );
    assert_eq!(read_only.state.lock().unwrap().streak, 1);
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Writer session is read-only (writes fail with SQLSTATE 25006); recreating Prisma client. reason=postgres_read_only_transaction"
        )),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn recreate_declines_inside_the_cooldown() {
    let (logs, guard) = capture_logs();
    let reconnect = Arc::new(FakeReconnect::default());
    let read_only = writer(&reconnect, COOLDOWN, RecreateState::default());
    assert!(read_only.recreate("first", None).await);
    let recreated = read_only.recreate("second", None).await;
    drop(guard);
    assert!(!recreated);
    assert_eq!(reconnect.called().len(), 1);
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Writer session still read-only after 1 recreate(s); backing off 30s. reason=second"
        )),
        "{:?}",
        logs.lines()
    );
}

#[rstest]
#[case::first(0)]
#[case::second(1)]
#[case::third(2)]
#[case::deep(5)]
#[tokio::test]
async fn recreate_waits_longer_with_each_streak(#[case] streak: u32) {
    let reconnect = Arc::new(FakeReconnect::default());
    let clock = fake_clock();
    let read_only = ReadOnlyWriter {
        reconnect: Arc::clone(&reconnect),
        cooldown: COOLDOWN,
        clock: clock.clone(),
        state: Mutex::new(RecreateState {
            last_recreate: Some(clock.now()),
            streak,
        }),
    };
    let wait = COOLDOWN * (1_u32 << streak);
    assert!(!read_only.recreate("waiting", None).await);
    clock.advance(wait.saturating_sub(Duration::from_millis(1)));
    assert!(!read_only.recreate("still waiting", None).await);
    clock.advance(Duration::from_millis(1));
    assert!(read_only.recreate("due", None).await);
    assert_eq!(reconnect.called().len(), 1);
}

#[rstest]
#[case::capped_by_the_exponent(1, Duration::from_secs(400))]
#[case::capped_by_a_deep_streak(20, COOLDOWN)]
#[tokio::test]
async fn recreate_caps_the_wait_at_six_hundred_seconds(
    #[case] streak: u32,
    #[case] cooldown: Duration,
) {
    let reconnect = Arc::new(FakeReconnect::default());
    let clock = fake_clock();
    let read_only = ReadOnlyWriter {
        reconnect: Arc::clone(&reconnect),
        cooldown,
        clock: clock.clone(),
        state: Mutex::new(RecreateState {
            last_recreate: Some(clock.now()),
            streak,
        }),
    };
    clock.advance(Duration::from_secs(599));
    assert!(!read_only.recreate("early", None).await);
    clock.advance(Duration::from_secs(1));
    assert!(read_only.recreate("due", None).await);
}

#[tokio::test]
async fn recreate_keeps_the_state_when_the_reconnect_declines() {
    let reconnect = Arc::new(FakeReconnect::default());
    reconnect.script_result(false);
    let read_only = writer(&reconnect, COOLDOWN, RecreateState::default());
    let recreated = read_only.recreate("declined", None).await;
    assert!(!recreated);
    let state = *read_only.state.lock().unwrap();
    assert_eq!(state.streak, 1);
    assert!(state.last_recreate.is_some());
}

#[tokio::test]
async fn reset_allows_the_next_recreate_immediately() {
    let reconnect = Arc::new(FakeReconnect::default());
    let read_only = writer(&reconnect, COOLDOWN, RecreateState::default());
    assert!(read_only.recreate("first", None).await);
    assert!(!read_only.recreate("second", None).await);
    read_only.reset();
    assert_eq!(*read_only.state.lock().unwrap(), RecreateState::default());
    assert!(read_only.recreate("after reset", None).await);
    assert_eq!(reconnect.called().len(), 2);
}

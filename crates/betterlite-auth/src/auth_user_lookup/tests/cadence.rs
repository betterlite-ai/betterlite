use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rstest::rstest;

use super::support::{ManualClock, ScriptedClock, cadence_entry, user_row};
use crate::auth_user_lookup::cadence::{Cadence, CadenceClock, WallClock};

fn cadence_with(max_size: usize) -> (Arc<ManualClock>, Cadence) {
    let clock = Arc::new(ManualClock::default());
    let hook: Arc<dyn CadenceClock> = Arc::<ManualClock>::clone(&clock);
    (clock, Cadence::new(max_size, hook))
}

#[test]
fn absent_cadence_reads() {
    let (_clock, cadence) = cadence_with(100);
    assert!(cadence.should_check_db("user_id:u1", 5));
    assert_eq!(cadence_entry(&cadence, "user_id:u1"), None);
}

#[test]
fn positive_cadence_always_reads() {
    let (clock, cadence) = cadence_with(100);
    cadence.update("user_id:u1", Some(user_row("u1")));
    assert!(cadence.should_check_db("user_id:u1", 5));
    clock.set(Duration::from_secs(10_000));
    assert!(cadence.should_check_db("user_id:u1", 5));
}

#[test]
fn negative_before_and_at_expiry() {
    let (clock, cadence) = cadence_with(100);
    clock.set(Duration::from_secs(100));
    cadence.update("user_id:u1", None);
    clock.set(Duration::from_secs(104));
    assert!(!cadence.should_check_db("user_id:u1", 5));
    clock.set(Duration::from_secs(105));
    assert!(cadence.should_check_db("user_id:u1", 5));
}

#[test]
fn backward_clock_keeps_negative() {
    let (clock, cadence) = cadence_with(100);
    clock.set(Duration::from_secs(100));
    cadence.update("user_id:u1", None);
    clock.set(Duration::from_secs(90));
    assert!(!cadence.should_check_db("user_id:u1", 5));
    clock.set(Duration::from_secs(100));
    assert!(!cadence.should_check_db("user_id:u1", 5));
}

#[test]
fn read_does_not_refresh() {
    let (clock, cadence) = cadence_with(100);
    clock.set(Duration::from_secs(100));
    cadence.update("user_id:u1", None);
    clock.set(Duration::from_secs(104));
    assert!(!cadence.should_check_db("user_id:u1", 5));
    assert!(!cadence.should_check_db("user_id:u1", 5));
    clock.set(Duration::from_secs(105));
    assert!(cadence.should_check_db("user_id:u1", 5));
}

#[rstest]
#[case::zero(0)]
#[case::negative_one(-1)]
#[case::negative_sixty(-60)]
fn zero_negative_ttl_expired(#[case] expiry: i64) {
    let (clock, cadence) = cadence_with(100);
    clock.set(Duration::from_secs(100));
    cadence.update("user_id:u1", None);
    assert!(
        cadence.should_check_db("user_id:u1", expiry),
        "expiry {expiry} expires a stored miss at once"
    );
}

#[test]
fn capacity_update_evicts_oldest() {
    let (_clock, cadence) = cadence_with(2);
    cadence.update("user_id:a", Some(user_row("a")));
    cadence.update("user_id:b", Some(user_row("b")));
    cadence.update("user_id:a", Some(user_row("new-a")));
    assert_eq!(
        cadence_entry(&cadence, "user_id:a").unwrap().record,
        Some(user_row("new-a"))
    );
    assert!(cadence_entry(&cadence, "user_id:b").is_some());
    cadence.update("user_id:c", None);
    assert!(
        cadence_entry(&cadence, "user_id:b").is_none(),
        "the entry left oldest by the update is evicted next"
    );
    assert!(cadence_entry(&cadence, "user_id:a").is_some());
    assert!(cadence_entry(&cadence, "user_id:c").is_some());
}

#[test]
fn update_samples_clock() {
    let clock: Arc<dyn CadenceClock> = Arc::new(ScriptedClock::new(
        [Duration::from_secs(100), Duration::from_secs(250)],
        Duration::from_secs(1_000),
    ));
    let cadence = Cadence::new(100, clock);
    cadence.update("user_id:a", None);
    cadence.update("user_id:b", None);
    assert_eq!(
        cadence_entry(&cadence, "user_id:a").unwrap().checked_at,
        Duration::from_secs(100)
    );
    assert_eq!(
        cadence_entry(&cadence, "user_id:b").unwrap().checked_at,
        Duration::from_secs(250)
    );
    assert!(cadence.should_check_db("user_id:a", 5));
}

#[test]
fn wall_clock_reads_the_system_time() {
    let sampled = WallClock.now();
    let system = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the system clock is after the epoch");
    assert!(
        system.abs_diff(sampled) < Duration::from_secs(60),
        "the wall clock samples the system time"
    );
}

#[test]
fn negative_expiry_uses_backward_movement_magnitude() {
    let (clock, cadence) = cadence_with(100);
    clock.set(Duration::from_secs(100));
    cadence.update("user_id:u1", None);
    clock.set(Duration::from_secs(97));
    assert!(
        cadence.should_check_db("user_id:u1", -5),
        "three seconds back is inside the five second magnitude"
    );
    assert!(
        !cadence.should_check_db("user_id:u1", 5),
        "a positive expiry never expires from backward movement"
    );
    clock.set(Duration::from_secs(95));
    assert!(
        cadence.should_check_db("user_id:u1", -5),
        "a movement equal to the magnitude expires the stored miss"
    );
    clock.set(Duration::from_secs(94));
    assert!(
        !cadence.should_check_db("user_id:u1", -5),
        "a movement past the magnitude is not expired"
    );
    assert!(
        !cadence.should_check_db("user_id:u1", 0),
        "a zero expiry leaves a backward clock unexpired"
    );
}

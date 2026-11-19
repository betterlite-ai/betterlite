use std::sync::Arc;
use std::time::Duration;

use rust_decimal::Decimal;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::support::{self, Call, Fault, Recorder};
use crate::auth_invalidation::AuthInvalidation;
use crate::auth_invalidation::registry::{InflightLoad, MembershipInflight};

const COUNTER: &str = "spend:team_member:user-1:team-1";
const FLOOR: &str = "spend_db_floor:spend:team_member:user-1:team-1";
const MEMBERSHIP_AUTH: &str = "team-1_user-1";
const RESERVATION: &str = "team_membership:user-1:team-1";

fn reset(recorder: &Arc<Recorder>) -> (AuthInvalidation, MembershipInflight<String>) {
    (
        support::builder(recorder, None),
        MembershipInflight::default(),
    )
}

fn eviction_call() -> Call {
    Call::Evict {
        keys: vec![MEMBERSHIP_AUTH.to_owned(), RESERVATION.to_owned()],
    }
}

fn counter_publish(value: f64) -> Call {
    Call::Publish {
        key: COUNTER.to_owned(),
        new_value: Some(value),
        ttl: Some(60.0),
    }
}

fn floor_publish(value: f64) -> Call {
    Call::Publish {
        key: FLOOR.to_owned(),
        new_value: Some(value),
        ttl: Some(5.0),
    }
}

fn counter_memory(value: f64) -> Call {
    Call::SpendSetMemory {
        key: COUNTER.to_owned(),
        value,
        ttl: 60.0,
    }
}

fn counter_redis(value: f64) -> Call {
    Call::SpendSetRedis {
        key: COUNTER.to_owned(),
        value,
        ttl: 60.0,
    }
}

fn floor_memory(value: f64) -> Call {
    Call::SpendSetMemory {
        key: FLOOR.to_owned(),
        value,
        ttl: 5.0,
    }
}

#[test]
fn member_counter_floor_and_membership_keys() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    runtime
        .block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::from(7)),
        ))
        .expect("the reset succeeds");
    assert_eq!(
        recorder.calls(),
        vec![
            counter_memory(7.0),
            counter_redis(7.0),
            floor_memory(7.0),
            counter_publish(7.0),
            floor_publish(7.0),
            eviction_call(),
        ]
    );
}

#[test]
fn cap_change_leaves_live_spend_and_floor() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    runtime
        .block_on(invalidation.invalidate_team_member_spend(&inflight, "user-1", "team-1", None))
        .expect("the cap change succeeds");
    assert_eq!(
        recorder.calls(),
        vec![eviction_call()],
        "a cap-only change never touches the live counter or the floor"
    );
}

#[test]
fn zero_reset_is_a_value() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    runtime
        .block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::ZERO),
        ))
        .expect("the reset succeeds");
    assert_eq!(
        recorder.calls(),
        vec![
            counter_memory(0.0),
            counter_redis(0.0),
            floor_memory(0.0),
            counter_publish(0.0),
            floor_publish(0.0),
            eviction_call(),
        ]
    );
}

#[test]
fn reset_memory_then_redis_ttl_60() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    runtime
        .block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::from(2)),
        ))
        .expect("the reset succeeds");
    let calls = recorder.calls();
    assert_eq!(calls[0], counter_memory(2.0));
    assert_eq!(calls[1], counter_redis(2.0));
}

#[test]
fn no_redis_still_updates_floor() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let (invalidation, inflight) = reset(&recorder);
    runtime
        .block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::from(4)),
        ))
        .expect("the reset succeeds");
    assert_eq!(
        recorder.calls(),
        vec![
            counter_memory(4.0),
            Call::SpendRedisSkipped(COUNTER.to_owned()),
            floor_memory(4.0),
            counter_publish(4.0),
            floor_publish(4.0),
            eviction_call(),
        ],
        "Redis absence skips only its direct write"
    );
}

#[test]
fn redis_set_failure_warns_then_delete() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    recorder.fail(Fault::SpendSetRedis);
    let (invalidation, inflight) = reset(&recorder);
    let (outcome, warnings) = support::capture(|| {
        runtime.block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::from(5)),
        ))
    });
    assert!(outcome.is_ok());
    assert_eq!(
        support::warned_subjects(&warnings),
        vec![COUNTER.to_owned()]
    );
    assert_eq!(
        recorder.calls(),
        vec![
            counter_memory(5.0),
            counter_redis(5.0),
            Call::SpendRedisDeleted(COUNTER.to_owned()),
            floor_memory(5.0),
            counter_publish(5.0),
            floor_publish(5.0),
            eviction_call(),
        ]
    );
}

#[test]
fn redis_delete_failure_503_after_memory_write() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    recorder.fail(Fault::SpendSetRedis);
    recorder.fail(Fault::SpendDeleteRedis);
    let (invalidation, inflight) = reset(&recorder);
    let (outcome, warnings) = support::capture(|| {
        runtime.block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::from(6)),
        ))
    });
    let error = outcome.expect_err("both Redis steps failing surfaces the reset failure");
    let message = error.to_string();
    assert!(
        message.contains("Retry once Redis is reachable"),
        "the 503 detail names Redis recovery: {message}"
    );
    assert_eq!(support::warned_subjects(&warnings).len(), 2);
    assert_eq!(
        recorder.calls(),
        vec![
            counter_memory(6.0),
            counter_redis(6.0),
            Call::SpendRedisDeleted(COUNTER.to_owned()),
        ],
        "the memory counter was written and nothing after it ran"
    );
}

#[test]
fn reset_503_retains_set_cause_and_skips_remaining_steps() {
    use std::error::Error as _;

    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    recorder.fail(Fault::SpendSetRedis);
    recorder.fail(Fault::SpendDeleteRedis);
    let (invalidation, inflight) = reset(&recorder);
    let outcome = runtime.block_on(invalidation.invalidate_team_member_spend(
        &inflight,
        "user-1",
        "team-1",
        Some(Decimal::from(8)),
    ));
    let error = outcome.expect_err("both Redis steps failing surfaces the reset failure");
    let cause = error.source().expect("the SET error stays the cause");
    assert!(
        cause.to_string().contains("does not fit a Redis expiry"),
        "the cause is the failed SET, not the failed DELETE: {cause}"
    );
    assert!(
        !recorder
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Publish { .. } | Call::Evict { .. })),
        "no publication or membership eviction happens after the 503"
    );
}

#[test]
fn floor_local_only_ttl_5() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    runtime
        .block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::from(9)),
        ))
        .expect("the reset succeeds");
    let calls = recorder.calls();
    assert!(calls.contains(&floor_memory(9.0)));
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, Call::SpendSetRedis { key, .. } if key.as_str() == FLOOR)),
        "the floor marker is memory-only"
    );
}

#[test]
fn counter_set_then_floor_set_publication() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    runtime
        .block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::from(3)),
        ))
        .expect("the reset succeeds");
    assert_eq!(
        recorder.calls(),
        vec![
            counter_memory(3.0),
            counter_redis(3.0),
            floor_memory(3.0),
            counter_publish(3.0),
            floor_publish(3.0),
            eviction_call(),
        ]
    );
}

#[test]
fn memberships_evicted_last() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    runtime
        .block_on(invalidation.invalidate_team_member_spend(
            &inflight,
            "user-1",
            "team-1",
            Some(Decimal::from(1)),
        ))
        .expect("the reset succeeds");
    let calls = recorder.calls();
    assert_eq!(calls.last(), Some(&eviction_call()));
    let Call::Evict { keys } = &calls[calls.len() - 1] else {
        panic!("the last call is the membership eviction");
    };
    assert_eq!(
        keys,
        &vec![MEMBERSHIP_AUTH.to_owned(), RESERVATION.to_owned()]
    );
}

#[test]
fn cancellation_not_warning_fallback() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    recorder.fail(Fault::SpendSetRedisPending);
    let (invalidation, inflight) = reset(&recorder);
    let (outcome, warnings) = support::capture(|| {
        runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_millis(50),
                invalidation.invalidate_team_member_spend(
                    &inflight,
                    "user-1",
                    "team-1",
                    Some(Decimal::from(1)),
                ),
            )
            .await
        })
    });
    assert!(
        outcome.is_err(),
        "cancellation is not turned into a warning"
    );
    assert!(warnings.is_empty());
}

#[test]
fn inflight_pop_before_any_mutation() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    let other_id = support::other_task_id();
    let (sender, receiver) = watch::channel(None::<String>);
    inflight.install(
        RESERVATION,
        InflightLoad {
            id: other_id,
            completion: receiver,
        },
    );
    let reset_future = invalidation.invalidate_team_member_spend(
        &inflight,
        "user-1",
        "team-1",
        Some(Decimal::from(2)),
    );
    let drive = async {
        tokio::task::yield_now().await;
        assert!(
            recorder.calls().is_empty(),
            "the reset waits for the membership load before any mutation"
        );
        sender
            .send(Some("loaded".to_owned()))
            .expect("the load still runs");
    };
    let (outcome, ()) = runtime.block_on(async { tokio::join!(reset_future, drive) });
    assert!(outcome.is_ok());
    assert!(inflight.get(RESERVATION).is_none());
    assert!(!recorder.calls().is_empty());
}

#[test]
fn inflight_wait_ignores_task_result() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let (invalidation, inflight) = reset(&recorder);
    let other_id = support::other_task_id();
    let (sender, receiver) = watch::channel(None::<String>);
    inflight.install(
        RESERVATION,
        InflightLoad {
            id: other_id,
            completion: receiver,
        },
    );
    let reset_future =
        invalidation.invalidate_team_member_spend(&inflight, "user-1", "team-1", None);
    let drive = async {
        tokio::task::yield_now().await;
        sender
            .send(Some("loaded".to_owned()))
            .expect("the load still runs");
    };
    let (outcome, ()) = runtime.block_on(async { tokio::join!(reset_future, drive) });
    assert!(
        outcome.is_ok(),
        "the wait never consumes or propagates the load's result"
    );
}

#[test]
fn current_or_missing_task_not_awaited() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let invalidation = Arc::new(support::builder(&recorder, None));
    let inflight = Arc::new(MembershipInflight::<String>::default());
    let missing = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_millis(50),
            invalidation.invalidate_team_member_spend(&inflight, "user-1", "team-1", None),
        )
        .await
    });
    assert!(missing.is_ok(), "a missing load never blocks the reset");
    let scoped = Arc::clone(&invalidation);
    let scoped_inflight = Arc::clone(&inflight);
    let joined = runtime.block_on(async move {
        let mut tasks = JoinSet::new();
        let _handle = tasks.spawn(async move {
            let id = tokio::task::try_id().expect("the reset runs inside its task");
            let (_sender, receiver) = watch::channel(None::<String>);
            scoped_inflight.install(
                RESERVATION,
                InflightLoad {
                    id,
                    completion: receiver,
                },
            );
            scoped
                .invalidate_team_member_spend(&scoped_inflight, "user-1", "team-1", None)
                .await
        });
        tokio::time::timeout(Duration::from_millis(50), tasks.join_next()).await
    });
    assert!(
        joined.is_ok(),
        "the reset never waits for its own membership load"
    );
}

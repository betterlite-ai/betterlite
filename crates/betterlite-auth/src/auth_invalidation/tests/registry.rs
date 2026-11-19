use tokio::sync::watch;

use super::support;
use crate::auth_invalidation::registry::{InflightLoad, MAX_MEMBERSHIP_LOADS, MembershipInflight};

#[test]
fn registry_bound_does_not_cancel_load() {
    let inflight = MembershipInflight::<u8>::default();
    let (sender, receiver) = watch::channel(None::<u8>);
    let id = support::other_task_id();
    inflight.install(
        "team_membership:user-1:team-0",
        InflightLoad {
            id,
            completion: receiver,
        },
    );
    for index in 1..=MAX_MEMBERSHIP_LOADS {
        let (_sender, receiver) = watch::channel(None::<u8>);
        inflight.install(
            &format!("team_membership:user-1:team-{index}"),
            InflightLoad {
                id,
                completion: receiver,
            },
        );
    }
    assert!(
        inflight.get("team_membership:user-1:team-0").is_none(),
        "the oldest load falls out of the 10000-entry index"
    );
    assert!(
        inflight
            .get(&format!(
                "team_membership:user-1:team-{MAX_MEMBERSHIP_LOADS}"
            ))
            .is_some(),
        "the newest load is still indexed"
    );
    let late = sender.subscribe();
    sender
        .send(Some(7_u8))
        .expect("the evicted load still runs");
    assert_eq!(*late.borrow(), Some(7_u8));
}

#[test]
fn registry_pop_returns_the_load_once() {
    let inflight = MembershipInflight::<u8>::default();
    let (_sender, receiver) = watch::channel(None::<u8>);
    inflight.install(
        "team_membership:user-1:team-1",
        InflightLoad {
            id: support::other_task_id(),
            completion: receiver,
        },
    );
    assert!(inflight.get("team_membership:user-1:team-1").is_some());
    assert!(inflight.pop("team_membership:user-1:team-1").is_some());
    assert!(inflight.pop("team_membership:user-1:team-1").is_none());
}

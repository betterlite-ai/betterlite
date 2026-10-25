#![cfg(test)]
use std::sync::Arc;
use std::time::Duration;

use futures_util::poll;
use rstest::rstest;
use tokio::sync::SetOnce;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::events::{Approval, Decision};
use crate::types::JsonObject;

fn approval() -> Approval {
    Approval {
        tool: "Bash".to_owned(),
        input: JsonObject::new(),
        decision: Arc::new(SetOnce::new()),
    }
}

#[tokio::test]
async fn an_answer_from_another_thread_wakes_the_waiting_turn() {
    let approval = approval();
    assert!(!approval.decision.initialized());
    let waiting = approval.decision.wait();
    tokio::pin!(waiting);
    assert!(poll!(waiting.as_mut()).is_pending());
    let answering = approval.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || answering.resolve(Decision::Allow));
    let decision = timeout(Duration::from_secs(1), waiting).await.unwrap();
    tasks.join_next().await.unwrap().unwrap();
    assert!(approval.decision.initialized());
    assert_eq!(decision, &Decision::Allow);
    approval.resolve(Decision::Deny {
        reason: "too late".to_owned(),
    });
    assert_eq!(approval.decision.wait().await, &Decision::Allow);
}

#[rstest]
#[case::allow(Decision::Allow)]
#[case::deny(Decision::Deny { reason: "no".to_owned() })]
#[tokio::test]
async fn the_first_answer_wins(#[case] first: Decision) {
    let approval = approval();
    assert!(!approval.decision.initialized());
    approval.resolve(first.clone());
    assert!(approval.decision.initialized());
    approval.resolve(Decision::Allow);
    approval.resolve(Decision::Deny {
        reason: "later denial".to_owned(),
    });
    let decision = timeout(Duration::from_secs(1), approval.decision.wait())
        .await
        .unwrap();
    assert_eq!(decision, &first);
}

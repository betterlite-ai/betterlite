#![cfg(test)]
use std::sync::Arc;
use std::thread;

use tokio::sync::SetOnce;

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
    let answering = approval.clone();
    let (decision, ()) = tokio::join!(approval.decision.wait(), async {
        thread::scope(|scope| {
            scope.spawn(|| answering.decision.set(Decision::Allow).unwrap());
        });
    });
    assert_eq!(decision, &Decision::Allow);
}

#[test]
fn the_first_answer_wins() {
    let approval = approval();
    assert!(!approval.decision.initialized());
    approval
        .decision
        .set(Decision::Deny {
            reason: "no".to_owned(),
        })
        .unwrap();
    assert!(approval.decision.set(Decision::Allow).is_err());
    assert!(approval.decision.initialized());
    assert_eq!(
        approval.decision.get(),
        Some(&Decision::Deny {
            reason: "no".to_owned()
        })
    );
}

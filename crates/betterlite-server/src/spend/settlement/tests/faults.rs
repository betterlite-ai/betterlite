use super::super::error::SpendSettlementError;
use super::journal::{
    Effect, Fault, Recorder, Request, WriteFailed, armed, forwarded, opened, reconciled,
};

#[tokio::test]
async fn a_charged_write_reconciles_first_then_counts_inside_one_batch() {
    let request = Request::new();
    let recorder = Recorder::failing(&[]);
    assert!(recorder.settle(&request.input(true)).await.unwrap());
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            armed(),
            forwarded(true),
            Effect::BatchClosed,
        ]
    );
    assert!(!request.finalized());
}

#[tokio::test]
async fn an_unreserved_request_reconciles_nothing_and_binds_the_post_call_keys() {
    let request = Request::new();
    let recorder = Recorder::failing(&[]);
    assert!(recorder.settle(&request.input(false)).await.unwrap());
    assert_eq!(
        recorder.effects(),
        vec![
            opened(false),
            Effect::Write,
            armed(),
            forwarded(false),
            Effect::BatchClosed,
        ]
    );
}

#[tokio::test]
async fn a_failed_reconcile_invalidates_finalizes_and_still_persists_the_spend() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Reconcile]);
    assert!(recorder.settle(&request.input(true)).await.unwrap());
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            Effect::Invalidate,
            opened(true),
            Effect::Write,
            armed(),
            forwarded(true),
            Effect::BatchClosed,
        ]
    );
    assert!(request.finalized());
}

#[tokio::test]
async fn a_failed_reconcile_finalizes_even_when_the_invalidation_fails() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Reconcile, Fault::Invalidate]);
    assert!(recorder.settle(&request.input(true)).await.unwrap());
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            Effect::Invalidate,
            opened(true),
            Effect::Write,
            armed(),
            forwarded(true),
            Effect::BatchClosed,
        ]
    );
    assert!(request.finalized());
}

#[tokio::test]
async fn an_uncharged_write_releases_the_reservation_and_answers_false() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::NotCharged]);
    assert!(!recorder.settle(&request.input(true)).await.unwrap());
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            Effect::Release,
            Effect::BatchClosed,
        ]
    );
    assert!(!request.finalized());
}

#[tokio::test]
async fn an_uncharged_unreserved_write_answers_false_without_cleanup() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::NotCharged]);
    assert!(!recorder.settle(&request.input(false)).await.unwrap());
    assert_eq!(
        recorder.effects(),
        vec![opened(false), Effect::Write, Effect::BatchClosed]
    );
}

#[tokio::test]
async fn a_failed_release_of_an_uncharged_write_propagates_without_invalidating() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::NotCharged, Fault::Release]);
    let outcome = recorder.settle(&request.input(true)).await;
    assert!(matches!(outcome, Err(SpendSettlementError::Release(_))));
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            Effect::Release,
            Effect::BatchClosed,
        ]
    );
    assert!(!request.finalized());
}

#[tokio::test]
async fn a_failed_write_releases_the_reservation_and_returns_the_write_error() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Write]);
    let outcome = recorder.settle(&request.input(true)).await;
    assert!(matches!(
        outcome,
        Err(SpendSettlementError::Write(WriteFailed))
    ));
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            Effect::Release,
            Effect::BatchClosed,
        ]
    );
    assert!(!request.finalized());
}

#[tokio::test]
async fn a_failed_write_and_release_invalidate_and_keep_the_write_error() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Write, Fault::Release]);
    let outcome = recorder.settle(&request.input(true)).await;
    assert!(matches!(
        outcome,
        Err(SpendSettlementError::Write(WriteFailed))
    ));
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            Effect::Release,
            Effect::Invalidate,
            Effect::BatchClosed,
        ]
    );
    assert!(!request.finalized());
}

#[tokio::test]
async fn cleanup_failures_never_replace_the_write_error() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Write, Fault::Release, Fault::Invalidate]);
    let outcome = recorder.settle(&request.input(true)).await;
    assert!(matches!(
        outcome,
        Err(SpendSettlementError::Write(WriteFailed))
    ));
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            Effect::Release,
            Effect::Invalidate,
            Effect::BatchClosed,
        ]
    );
}

#[tokio::test]
async fn a_failed_unreserved_write_returns_the_write_error_without_cleanup() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Write]);
    let outcome = recorder.settle(&request.input(false)).await;
    assert!(matches!(
        outcome,
        Err(SpendSettlementError::Write(WriteFailed))
    ));
    assert_eq!(
        recorder.effects(),
        vec![opened(false), Effect::Write, Effect::BatchClosed]
    );
}

#[tokio::test]
async fn a_failed_prefetch_after_the_write_skips_the_counters_and_leaves_the_reservation_open() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Prefetch]);
    let outcome = recorder.settle(&request.input(true)).await;
    assert!(matches!(outcome, Err(SpendSettlementError::Prefetch(_))));
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            armed(),
            Effect::BatchClosed,
        ]
    );
    assert!(!request.finalized());
}

#[tokio::test]
async fn a_failed_increment_invalidates_finalizes_and_never_writes_again() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Increment]);
    let outcome = recorder.settle(&request.input(true)).await;
    assert!(matches!(outcome, Err(SpendSettlementError::Counters(_))));
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            armed(),
            forwarded(true),
            Effect::Invalidate,
            Effect::BatchClosed,
        ]
    );
    assert!(request.finalized());
}

#[tokio::test]
async fn a_failed_increment_finalizes_even_when_the_invalidation_fails() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Increment, Fault::Invalidate]);
    let outcome = recorder.settle(&request.input(true)).await;
    assert!(matches!(outcome, Err(SpendSettlementError::Counters(_))));
    assert_eq!(
        recorder.effects(),
        vec![
            reconciled(),
            opened(true),
            Effect::Write,
            armed(),
            forwarded(true),
            Effect::Invalidate,
            Effect::BatchClosed,
        ]
    );
    assert!(request.finalized());
}

#[tokio::test]
async fn a_failed_unreserved_increment_rethrows_without_cleanup() {
    let request = Request::new();
    let recorder = Recorder::failing(&[Fault::Increment]);
    let outcome = recorder.settle(&request.input(false)).await;
    assert!(matches!(outcome, Err(SpendSettlementError::Counters(_))));
    assert_eq!(
        recorder.effects(),
        vec![
            opened(false),
            Effect::Write,
            armed(),
            forwarded(false),
            Effect::BatchClosed,
        ]
    );
}

#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rstest::rstest;

use crate::dispatch::error::DispatchError;
use crate::dispatch::tests::support::{
    FakeEmbedding, MODEL_ID, RecordingLog, RecordingRules, Trace, context, deployment,
    embedding_request,
};
use crate::dispatch::{Dispatch, NoRules};
use crate::operation::{CallResult, Operation};
use crate::rules::RuleDecision;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    LifecycleOrder,
    RulesSkipped,
    DeadlineCapsTimeout,
    ExpiredDeadline,
    MissingDeployment,
}

#[rstest]
#[case::lifecycle_order(Scenario::LifecycleOrder)]
#[case::rules_skipped(Scenario::RulesSkipped)]
#[case::deadline_caps_timeout(Scenario::DeadlineCapsTimeout)]
#[case::expired_deadline(Scenario::ExpiredDeadline)]
#[case::missing_deployment(Scenario::MissingDeployment)]
#[tokio::test]
async fn call_dispatch_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::LifecycleOrder => lifecycle_order().await,
        Scenario::RulesSkipped => rules_skipped().await,
        Scenario::DeadlineCapsTimeout => deadline_caps_timeout().await,
        Scenario::ExpiredDeadline => {
            refused_before_transport(Some(SystemTime::UNIX_EPOCH), MODEL_ID, |error| {
                matches!(error, DispatchError::DeadlineExpired { .. })
            })
            .await;
        }
        Scenario::MissingDeployment => {
            refused_before_transport(None, "missing", |error| {
                matches!(error, DispatchError::MissingDeployment { .. })
            })
            .await;
        }
    }
}

async fn lifecycle_order() {
    let trace = Trace::default();
    let calls = Arc::new(FakeEmbedding::new(trace.clone()));
    let log = Arc::new(RecordingLog::new(trace.clone()));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(NoRules),
        log,
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let result = dispatch
        .execute(
            Operation::Embedding(embedding_request("embed-1", MODEL_ID, calls)),
            &context,
        )
        .await
        .expect("embedding call");
    let CallResult::Embedding(answer) = result else {
        panic!("an embedding call answers embeddings");
    };
    assert_eq!(answer.response.data.len(), 1);
    assert_eq!(trace.steps(), ["embed", "success"]);
}

async fn rules_skipped() {
    let trace = Trace::default();
    let calls = Arc::new(FakeEmbedding::new(trace.clone()));
    let log = Arc::new(RecordingLog::new(trace.clone()));
    let rules = Arc::new(RecordingRules::new(
        trace.clone(),
        Some(RuleDecision::Boolean(false)),
    ));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        log,
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    dispatch
        .execute(
            Operation::Embedding(embedding_request("embed-2", MODEL_ID, calls)),
            &context,
        )
        .await
        .expect("embedding call");
    assert_eq!(trace.steps(), ["embed", "success"]);
}

async fn deadline_caps_timeout() {
    let trace = Trace::default();
    let calls = Arc::new(FakeEmbedding::new(trace.clone()));
    let log = Arc::new(RecordingLog::new(trace));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(NoRules),
        log,
    );
    let deadline = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(3))
        .expect("deadline");
    let context = context(Some(deadline), SystemTime::UNIX_EPOCH);
    dispatch
        .execute(
            Operation::Embedding(embedding_request(
                "embed-3",
                MODEL_ID,
                Arc::<FakeEmbedding>::clone(&calls),
            )),
            &context,
        )
        .await
        .expect("bounded call");
    assert_eq!(calls.seen(), [Duration::from_secs(3)]);
}

async fn refused_before_transport(
    deadline: Option<SystemTime>,
    model_id: &str,
    expected: fn(&DispatchError) -> bool,
) {
    let trace = Trace::default();
    let calls = Arc::new(FakeEmbedding::new(trace.clone()));
    let log = Arc::new(RecordingLog::new(trace.clone()));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(NoRules),
        log,
    );
    let context = context(deadline, SystemTime::UNIX_EPOCH);
    let Err(error) = dispatch
        .execute(
            Operation::Embedding(embedding_request(
                "embed-4",
                model_id,
                Arc::<FakeEmbedding>::clone(&calls),
            )),
            &context,
        )
        .await
    else {
        panic!("the call must fail before transport");
    };
    assert!(expected(&error), "unexpected error: {error}");
    assert_eq!(trace.steps(), ["failure"]);
    assert!(calls.seen().is_empty());
}

#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use betterlite_openai::text_completion_request::CompletionRequest;
use betterlite_openai::text_completion_response::CompletionResponse;
use http::HeaderMap;
use rstest::rstest;

use crate::dispatch::Dispatch;
use crate::dispatch::error::DispatchError;
use crate::dispatch::log::CallOutcome;
use crate::dispatch::tests::support::{
    MODEL_ID, RecordingLog, RecordingRules, Trace, context, deployment,
};
use crate::operation::Entry;
use crate::operation::text_completion::{
    TextCompletionCall, TextCompletionCallInput, TextCompletionFailure, TextCompletionRequest,
    TextCompletionResult,
};
use crate::rules::RuleDecision;

struct FakeCompletion {
    trace: Trace,
    seen: Arc<Mutex<Vec<Duration>>>,
    failure: Option<TextCompletionFailure>,
}

impl FakeCompletion {
    fn new(trace: Trace) -> Self {
        Self {
            trace,
            seen: Arc::new(Mutex::new(Vec::new())),
            failure: None,
        }
    }

    fn failing(trace: Trace, failure: TextCompletionFailure) -> Self {
        Self {
            trace,
            seen: Arc::new(Mutex::new(Vec::new())),
            failure: Some(failure),
        }
    }

    fn seen(&self) -> Vec<Duration> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl TextCompletionCall for FakeCompletion {
    async fn complete(
        &self,
        call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        self.trace.push("complete");
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.deployment.timeout);
        if let Some(failure) = &self.failure {
            return Err(TextCompletionFailure {
                status: failure.status,
                error_type: failure.error_type.clone(),
                param: failure.param.clone(),
                message: failure.message.clone(),
            });
        }
        let response: CompletionResponse = serde_json::from_str(
            r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"up","choices":[],"usage":null}"#,
        )
        .expect("completion response");
        Ok(TextCompletionResult {
            response: Box::new(response),
            upstream_headers: HeaderMap::new(),
            upstream_time: Duration::ZERO,
        })
    }
}

fn request(
    call_id: &str,
    model_id: &str,
    dispatch: Arc<Dispatch>,
    calls: Arc<dyn TextCompletionCall>,
) -> TextCompletionRequest {
    let body = br#"{"model":"g","prompt":"hello"}"#;
    TextCompletionRequest {
        call_id: call_id.to_owned(),
        model_id: model_id.to_owned(),
        request: Box::new(CompletionRequest::parse(body).expect("completion request")),
        dispatch,
        calls,
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    LifecycleOrder,
    RulesReject,
    DeadlineCapsTimeout,
    ExpiredDeadline,
    MissingDeployment,
    FailureBecomesRejected,
}

#[rstest]
#[case::lifecycle_order(Scenario::LifecycleOrder)]
#[case::rules_reject(Scenario::RulesReject)]
#[case::deadline_caps_timeout(Scenario::DeadlineCapsTimeout)]
#[case::expired_deadline(Scenario::ExpiredDeadline)]
#[case::missing_deployment(Scenario::MissingDeployment)]
#[case::failure_becomes_rejected(Scenario::FailureBecomesRejected)]
#[tokio::test]
async fn call_dispatch_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::LifecycleOrder => lifecycle_order().await,
        Scenario::RulesReject => rules_reject().await,
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
        Scenario::FailureBecomesRejected => failure_becomes_rejected().await,
    }
}

async fn lifecycle_order() {
    let trace = Trace::default();
    let calls = Arc::new(FakeCompletion::new(trace.clone()));
    let log = Arc::new(RecordingLog::new(trace.clone()));
    let dispatch = Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(crate::dispatch::NoRules),
        log.clone(),
    ));
    let context = context(None, SystemTime::UNIX_EPOCH);
    dispatch
        .execute(
            crate::operation::Operation::TextCompletion(request(
                "completion-1",
                MODEL_ID,
                Arc::clone(&dispatch),
                calls.clone(),
            )),
            &context,
        )
        .await
        .expect("completion call");
    assert_eq!(trace.steps(), ["complete", "success"]);
    let recorded = log.calls();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].entry, Entry::TextCompletion);
    assert_eq!(recorded[0].outcome, CallOutcome::Succeeded);
}

async fn rules_reject() {
    let trace = Trace::default();
    let calls = Arc::new(FakeCompletion::new(trace.clone()));
    let log = Arc::new(RecordingLog::new(trace.clone()));
    let rules = Arc::new(RecordingRules::new(
        trace.clone(),
        Some(RuleDecision::Boolean(false)),
    ));
    let dispatch = Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        log,
    ));
    let context = context(None, SystemTime::UNIX_EPOCH);
    let error = dispatch
        .execute(
            crate::operation::Operation::TextCompletion(request(
                "completion-2",
                MODEL_ID,
                Arc::clone(&dispatch),
                calls.clone(),
            )),
            &context,
        )
        .await
        .expect_err("the rule must reject");
    assert!(matches!(error, DispatchError::Rules { .. }));
    assert_eq!(trace.steps(), ["rules", "failure"]);
    assert!(calls.seen().is_empty());
}

async fn deadline_caps_timeout() {
    let trace = Trace::default();
    let calls = Arc::new(FakeCompletion::new(trace.clone()));
    let log = Arc::new(RecordingLog::new(trace));
    let dispatch = Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(crate::dispatch::NoRules),
        log,
    ));
    let deadline = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(3))
        .expect("deadline");
    let context = context(Some(deadline), SystemTime::UNIX_EPOCH);
    dispatch
        .execute(
            crate::operation::Operation::TextCompletion(request(
                "completion-3",
                MODEL_ID,
                Arc::clone(&dispatch),
                Arc::clone(&calls),
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
    let calls = Arc::new(FakeCompletion::new(trace.clone()));
    let log = Arc::new(RecordingLog::new(trace));
    let dispatch = Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(crate::dispatch::NoRules),
        log,
    ));
    let context = context(deadline, SystemTime::UNIX_EPOCH);
    let error = dispatch
        .execute(
            crate::operation::Operation::TextCompletion(request(
                "completion-4",
                model_id,
                Arc::clone(&dispatch),
                Arc::clone(&calls),
            )),
            &context,
        )
        .await
        .expect_err("the call must fail before transport");
    assert!(expected(&error), "unexpected error: {error}");
    assert!(calls.seen().is_empty());
}

async fn failure_becomes_rejected() {
    let trace = Trace::default();
    let calls = Arc::new(FakeCompletion::failing(
        trace.clone(),
        TextCompletionFailure {
            status: 429,
            error_type: String::from("rate_limit_error"),
            param: Some(String::from("prompt")),
            message: String::from("slow down"),
        },
    ));
    let log = Arc::new(RecordingLog::new(trace.clone()));
    let dispatch = Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        Arc::new(crate::dispatch::NoRules),
        log,
    ));
    let context = context(None, SystemTime::UNIX_EPOCH);
    let error = dispatch
        .execute(
            crate::operation::Operation::TextCompletion(request(
                "completion-6",
                MODEL_ID,
                Arc::clone(&dispatch),
                calls,
            )),
            &context,
        )
        .await
        .expect_err("the provider failed");
    assert!(matches!(
        error,
        DispatchError::Rejected {
            status: 429,
            ref error_type,
            ..
        } if error_type == "rate_limit_error"
    ));
    assert_eq!(trace.steps(), ["complete", "failure"]);
}

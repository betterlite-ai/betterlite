#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use betterlite_openai::moderation_request::ModerationRequest as ClientModerationRequest;
use betterlite_openai::moderation_response::ModerationResponse;
use http::HeaderMap;
use rstest::rstest;

use crate::dispatch::Dispatch;
use crate::dispatch::error::DispatchError;
use crate::dispatch::tests::support::{
    MODEL_ID, RecordingLog, RecordingRules, Trace, context, deployment,
};
use crate::operation::moderation::{
    ModerationCall, ModerationCallInput, ModerationFailure, ModerationRequest, ModerationResult,
};
use crate::operation::{CallResult, Operation};
use crate::rules::RuleDecision;

const BODY: &[u8] = br#"{"input":"hello","model":"g"}"#;

#[derive(Clone, Copy, Debug)]
enum Answer {
    Succeeds,
    DecimalScore,
    MissingKey,
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    LifecycleOrder,
    FailureKeepsItsFields,
    NullTypeTravelsEmpty,
    RulesSkipped,
    DeadlineCapsTimeout,
    ExpiredDeadline,
    MissingDeployment,
}

struct FakeModeration {
    trace: Trace,
    seen: Arc<Mutex<Vec<Duration>>>,
    answer: Answer,
}

impl FakeModeration {
    fn new(trace: Trace, answer: Answer) -> Self {
        Self {
            trace,
            seen: Arc::new(Mutex::new(Vec::new())),
            answer,
        }
    }
}

#[async_trait]
impl ModerationCall for FakeModeration {
    async fn moderate(
        &self,
        call: ModerationCallInput<'_>,
    ) -> Result<ModerationResult, ModerationFailure> {
        self.trace.push("moderate");
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.deployment.timeout);
        match self.answer {
            Answer::Succeeds => Ok(ModerationResult {
                response: moderation_response(),
                upstream_headers: HeaderMap::new(),
                upstream_time: Duration::ZERO,
            }),
            Answer::DecimalScore => Err(ModerationFailure {
                status: 502,
                error_type: Some(String::from("bad_gateway")),
                param: Some(String::from("category_scores")),
                code: Some(String::from("decimal_out_of_range")),
                message: String::from("the score is out of range"),
            }),
            Answer::MissingKey => Err(ModerationFailure {
                status: 401,
                error_type: None,
                param: None,
                code: None,
                message: String::from("no API key"),
            }),
        }
    }
}

fn moderation_request(
    call_id: &str,
    model_id: &str,
    calls: Arc<dyn ModerationCall>,
    dispatch: &Arc<Dispatch>,
) -> ModerationRequest {
    ModerationRequest {
        call_id: call_id.to_owned(),
        model_id: model_id.to_owned(),
        request: Box::new(ClientModerationRequest::parse(BODY).expect("moderation request")),
        dispatch: Arc::clone(dispatch),
        calls,
    }
}

fn moderation_response() -> ModerationResponse {
    ModerationResponse::parse(br#"{"id":"modr-1","model":"omni-moderation-latest","results":[]}"#)
        .expect("moderation response")
}

fn recording_dispatch(trace: &Trace, rules: Arc<RecordingRules>) -> Arc<Dispatch> {
    Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        Arc::new(RecordingLog::new(trace.clone())),
    ))
}

#[rstest]
#[case::lifecycle_order(Scenario::LifecycleOrder)]
#[case::failure_keeps_its_fields(Scenario::FailureKeepsItsFields)]
#[case::null_type_travels_empty(Scenario::NullTypeTravelsEmpty)]
#[case::rules_skipped(Scenario::RulesSkipped)]
#[case::deadline_caps_timeout(Scenario::DeadlineCapsTimeout)]
#[case::expired_deadline(Scenario::ExpiredDeadline)]
#[case::missing_deployment(Scenario::MissingDeployment)]
#[tokio::test]
async fn moderations_moderation_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::LifecycleOrder => lifecycle_order().await,
        Scenario::FailureKeepsItsFields => failure_keeps_its_fields().await,
        Scenario::NullTypeTravelsEmpty => null_type_travels_empty().await,
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
    let calls = Arc::new(FakeModeration::new(trace.clone(), Answer::Succeeds));
    let dispatch = recording_dispatch(&trace, Arc::new(RecordingRules::new(trace.clone(), None)));
    let context = context(None, SystemTime::UNIX_EPOCH);
    let result = dispatch
        .execute(
            Operation::Moderation(moderation_request("mod-1", MODEL_ID, calls, &dispatch)),
            &context,
        )
        .await
        .expect("moderation call");
    let CallResult::Moderation(answer) = result else {
        panic!("a moderation call answers moderation");
    };
    assert_eq!(answer.response.id, "modr-1");
    assert_eq!(trace.steps(), ["moderate", "success"]);
}

async fn failure_keeps_its_fields() {
    let trace = Trace::default();
    let calls = Arc::new(FakeModeration::new(trace.clone(), Answer::DecimalScore));
    let dispatch = recording_dispatch(&trace, Arc::new(RecordingRules::new(trace.clone(), None)));
    let context = context(None, SystemTime::UNIX_EPOCH);
    let error = refused(dispatch.execute(
        Operation::Moderation(moderation_request("mod-2", MODEL_ID, calls, &dispatch)),
        &context,
    ))
    .await;
    let DispatchError::Rejected {
        status,
        error_type,
        param,
        code,
        message,
    } = error
    else {
        panic!("a provider failure is a rejected call");
    };
    assert_eq!(status, 502);
    assert_eq!(error_type, "bad_gateway");
    assert_eq!(param.as_deref(), Some("category_scores"));
    assert_eq!(code, "decimal_out_of_range");
    assert_eq!(message, "the score is out of range");
    assert_eq!(trace.steps(), ["moderate", "failure"]);
}

async fn null_type_travels_empty() {
    let trace = Trace::default();
    let calls = Arc::new(FakeModeration::new(trace.clone(), Answer::MissingKey));
    let dispatch = recording_dispatch(&trace, Arc::new(RecordingRules::new(trace.clone(), None)));
    let context = context(None, SystemTime::UNIX_EPOCH);
    let error = refused(dispatch.execute(
        Operation::Moderation(moderation_request("mod-3", MODEL_ID, calls, &dispatch)),
        &context,
    ))
    .await;
    let DispatchError::Rejected {
        status,
        error_type,
        param,
        code,
        ..
    } = error
    else {
        panic!("a provider failure is a rejected call");
    };
    assert_eq!(status, 401);
    assert_eq!(error_type, "");
    assert_eq!(param, None);
    assert_eq!(code, "401");
}

async fn rules_skipped() {
    let trace = Trace::default();
    let calls = Arc::new(FakeModeration::new(trace.clone(), Answer::Succeeds));
    let rules = Arc::new(RecordingRules::new(
        trace.clone(),
        Some(RuleDecision::Boolean(false)),
    ));
    let dispatch = recording_dispatch(&trace, rules);
    let context = context(None, SystemTime::UNIX_EPOCH);
    dispatch
        .execute(
            Operation::Moderation(moderation_request("mod-4", MODEL_ID, calls, &dispatch)),
            &context,
        )
        .await
        .expect("moderation call");
    assert_eq!(trace.steps(), ["moderate", "success"]);
}

async fn deadline_caps_timeout() {
    let trace = Trace::default();
    let calls = Arc::new(FakeModeration::new(trace.clone(), Answer::Succeeds));
    let seen = Arc::clone(&calls.seen);
    let dispatch = recording_dispatch(&trace, Arc::new(RecordingRules::new(trace.clone(), None)));
    let deadline = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(3))
        .expect("deadline");
    let context = context(Some(deadline), SystemTime::UNIX_EPOCH);
    dispatch
        .execute(
            Operation::Moderation(moderation_request("mod-5", MODEL_ID, calls, &dispatch)),
            &context,
        )
        .await
        .expect("bounded call");
    let bounded = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(bounded, [Duration::from_secs(3)]);
}

async fn refused_before_transport(
    deadline: Option<SystemTime>,
    model_id: &str,
    expected: fn(&DispatchError) -> bool,
) {
    let trace = Trace::default();
    let calls = Arc::new(FakeModeration::new(trace.clone(), Answer::Succeeds));
    let dispatch = recording_dispatch(&trace, Arc::new(RecordingRules::new(trace.clone(), None)));
    let context = context(deadline, SystemTime::UNIX_EPOCH);
    let error = refused(dispatch.execute(
        Operation::Moderation(moderation_request("mod-6", model_id, calls, &dispatch)),
        &context,
    ))
    .await;
    assert!(expected(&error), "unexpected error: {error}");
    assert_eq!(trace.steps(), ["failure"]);
}

async fn refused<F, T>(future: F) -> DispatchError
where
    F: Future<Output = Result<T, DispatchError>>,
{
    let Err(error) = future.await else {
        panic!("the moderation call must fail");
    };
    error
}

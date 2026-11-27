use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::support::{
    Failure, Fixture, Recorder, RecordingConfig, awaited_create, awaited_list, caller, create_args,
    error, json, kwargs, list_args, object, run_fixture, runs_fixture,
};
use crate::api::eval_http::exchange::EvalLogging;
use crate::api::eval_run_create_list::providers::RunMode;
use crate::api::eval_run_create_list::{CreateRunWork, ListRunsWork};

// the flag makes the blocking entry hand its caller the handler's future instead of polling it
#[test]
fn create_with_the_flag_returns_the_pending_work() {
    let params = kwargs(r#"{"acreate_run":true}"#);
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);
    let calls = caller(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Deferred),
    );
    let mut fixture = Fixture::new("create_run");
    let env = fixture.env.pre_post();
    let log = EvalLogging {
        state: &mut fixture.state,
        env: &env,
    };

    let work = calls.create_run(request, log).expect("the call prepares");

    assert!(matches!(&work, CreateRunWork::Pending(_)));
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.mode, RunMode::Async);
}

#[test]
fn list_with_the_flag_returns_the_pending_work() {
    let params = kwargs(r#"{"alist_runs":true}"#);
    let request = list_args("eval-1", &params);
    let calls = caller(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Deferred),
    );
    let mut fixture = Fixture::new("list_runs");
    let env = fixture.env.pre_post();
    let log = EvalLogging {
        state: &mut fixture.state,
        env: &env,
    };

    let work = calls.list_runs(request, log).expect("the call prepares");

    assert!(matches!(&work, ListRunsWork::Pending(_)));
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.mode, RunMode::Async);
}

// acreate_run writes the flag and awaits the handler future, so a handler that answers late still answers
#[test]
fn acreate_run_awaits_a_handler_that_answers_late() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, calls) = awaited_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Deferred),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.mode, RunMode::Async);
}

#[test]
fn alist_runs_awaits_a_handler_that_answers_late() {
    let params = kwargs("{}");
    let request = list_args("eval-1", &params);

    let (answer, _fixture, calls) = awaited_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Deferred),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.mode, RunMode::Async);
}

// an awaited handler failure is mapped by the async wrapper with the flagged kwargs it called with
#[test]
fn acreate_run_maps_an_awaited_handler_error() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, _calls) = awaited_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Provider),
        request,
    );

    let failure = error(answer);
    assert!(matches!(&failure.class, ErrorClass::RateLimit(_)));
    assert_eq!(
        Json::Object(failure.local_vars.as_ref().clone()),
        json(
            r#"{"eval_id":"eval-1","data_source":{"type":"jsonl"},"name":null,"metadata":null,"extra_headers":null,"extra_query":null,"extra_body":null,"timeout":null,"custom_llm_provider":null,"kwargs":{"acreate_run":true}}"#
        )
    );
}

#[test]
fn alist_runs_maps_an_awaited_handler_error() {
    let params = kwargs("{}");
    let request = list_args("eval-1", &params);

    let (answer, _fixture, _calls) = awaited_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Provider),
        request,
    );

    let failure = error(answer);
    assert!(matches!(&failure.class, ErrorClass::RateLimit(_)));
    assert_eq!(
        Json::Object(failure.local_vars.as_ref().clone()),
        json(
            r#"{"eval_id":"eval-1","limit":null,"after":null,"before":null,"order":null,"extra_headers":null,"extra_query":null,"timeout":null,"custom_llm_provider":null,"kwargs":{"alist_runs":true}}"#
        )
    );
}

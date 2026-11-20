use std::sync::Arc;

use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::worker::{Admission, Callback, Rejection};

use super::support::Stub;
use super::support::capture_logs;
use super::support::chat_request;
use super::support::fixture;
use super::support::moment;
use super::support::request;
use super::support::response;
use super::support::result_label;
use super::support::saw_log;
use super::support::value;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::logging::{
    AsyncSuccessLog, CallLogging, DispatchFlags, SUCCESS_LOG_JOB, SuccessLogInput,
    SuccessLogScheduler, dispatch_success_logging, success_result,
};
use crate::call_wrapper::ports::CallTimes;
use crate::call_wrapper::state::WrapperOptions;

struct RejectingScheduler;

impl SuccessLogScheduler for RejectingScheduler {
    fn schedule(&self, _job: AsyncSuccessLog) -> Admission {
        Admission::Rejected(Rejection::QueueFull)
    }
}

fn sync_events(events: &[String]) -> usize {
    events
        .iter()
        .filter(|event| event.as_str() == "sync_callbacks:chat")
        .count()
}

#[tokio::test]
async fn one_handle_gets_one_scheduled_log() {
    let mut fixture = fixture();
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the first call succeeds");
    let handle = stub.handle(0);
    fixture
        .wrapper
        .call(
            &stub,
            chat_request("{}"),
            WrapperOptions {
                logging: Some(handle),
                internal_call: false,
            },
        )
        .await
        .expect("the second call succeeds");
    assert_eq!(fixture.scheduler.count(), 1);
    assert_eq!(sync_events(&fixture.logging.events()), 2);
}

#[tokio::test]
async fn the_completion_with_fallbacks_parent_is_skipped() {
    let mut fixture = fixture();
    let stub = Stub::new("acompletion");
    let request = chat_request(r#"{"fallbacks":["gpt-3.5-turbo"]}"#);
    fixture
        .wrapper
        .call(&stub, request, WrapperOptions::default())
        .await
        .expect("the call succeeds");
    let mut jobs = fixture.scheduler.take();
    assert_eq!(jobs.len(), 1);
    let job = jobs.remove(0);
    job.deliver().await;
    assert_eq!(
        fixture.logging.events(),
        vec![String::from("sync_callbacks:chat")]
    );
}

#[tokio::test]
async fn a_deferred_log_waits_for_the_stream_to_end() {
    let mut fixture = fixture();
    let request = chat_request(r#"{"litellm_call_id":"call-1"}"#);
    let (handle, _prepared) = fixture
        .wrapper
        .start_logging("acompletion", request, true)
        .expect("the logging state builds");
    {
        let mut state = handle.state.lock().expect("the state lock is free");
        state.deferred.defer_async_logging = true;
    };
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(
            &stub,
            chat_request("{}"),
            WrapperOptions {
                logging: Some(handle.clone()),
                internal_call: false,
            },
        )
        .await
        .expect("the call succeeds");
    assert_eq!(fixture.scheduler.count(), 0);
    let job = handle
        .deferred
        .lock()
        .expect("the deferred lock is free")
        .take()
        .expect("the log was parked");
    job.deliver().await;
    assert_eq!(
        fixture.logging.events(),
        vec![
            String::from("sync_callbacks:chat"),
            String::from("async_success:chat")
        ]
    );
}

#[tokio::test]
async fn an_internal_call_runs_the_sync_callbacks_only() {
    let mut fixture = fixture();
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(
            &stub,
            chat_request("{}"),
            WrapperOptions {
                logging: None,
                internal_call: true,
            },
        )
        .await
        .expect("the call succeeds");
    assert_eq!(fixture.scheduler.count(), 0);
    assert_eq!(
        fixture.logging.events(),
        vec![String::from("sync_callbacks:chat")]
    );
}

#[tokio::test]
async fn an_ordinary_job_delivers_the_async_success_log() {
    let mut fixture = fixture();
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the call succeeds");
    let mut jobs = fixture.scheduler.take();
    let job = jobs.remove(0);
    job.deliver().await;
    assert_eq!(
        fixture.logging.events(),
        vec![
            String::from("sync_callbacks:chat"),
            String::from("async_success:chat")
        ]
    );
}

#[tokio::test]
async fn the_job_is_named_like_the_litellm_success_handler() {
    let mut fixture = fixture();
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the call succeeds");
    let mut jobs = fixture.scheduler.take();
    let job = jobs.remove(0);
    assert_eq!(job.name(), SUCCESS_LOG_JOB);
}

#[tokio::test]
async fn the_jobs_call_delivers_the_async_success_log() {
    let mut fixture = fixture();
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the call succeeds");
    let mut jobs = fixture.scheduler.take();
    let job = jobs.remove(0);
    job.call().await.expect("the job always succeeds");
    assert_eq!(
        fixture.logging.events(),
        vec![
            String::from("sync_callbacks:chat"),
            String::from("async_success:chat")
        ]
    );
}

#[test]
fn a_non_chat_response_is_the_other_result() {
    let outcome = CallOutcome::Response(value(r#"{"id":"img-1"}"#));
    assert_eq!(
        result_label(&success_result(&outcome, "aimg_generation")),
        "other"
    );
    assert_eq!(
        result_label(&success_result(&outcome, "aembedding")),
        "embeddings"
    );
}

#[test]
fn a_rejected_success_log_is_warned_about() {
    let (events, _guard) = capture_logs();
    let mut fixture = fixture();
    let (handle, _prepared) = fixture
        .wrapper
        .start_logging(
            "acompletion",
            chat_request(r#"{"litellm_call_id":"call-1"}"#),
            true,
        )
        .expect("the logging state builds");
    let logging: Arc<dyn CallLogging> = fixture.logging.clone();
    let scheduler: Arc<dyn SuccessLogScheduler> = Arc::new(RejectingScheduler);
    dispatch_success_logging(
        &logging,
        &scheduler,
        &handle,
        SuccessLogInput {
            result: SuccessResult::Other(value("{}")),
            times: CallTimes {
                start: moment(),
                end: moment(),
            },
            flags: DispatchFlags {
                is_completion_with_fallbacks: false,
                is_internal_call: false,
            },
        },
    );
    assert!(saw_log(&events, "the async success log was dropped"));
}

#[tokio::test]
async fn async_embedding_without_a_partial_hit_updates_the_metadata() {
    let mut fixture = fixture();
    let body = value(r#"{"data":[[1,2]]}"#);
    let stub = Stub::responding("aembedding", response(body.clone()));
    let request = request(r#"{"model":"text-embedding-3-small","input":"hi"}"#);
    let outcome = fixture
        .wrapper
        .call(&stub, request, WrapperOptions::default())
        .await
        .expect("the embedding answers");
    assert_eq!(outcome, response(body));
    assert_eq!(fixture.metadata.count(), 1);
}

use super::support::Stub;
use super::support::chat_request;
use super::support::fixture;
use crate::call_wrapper::state::WrapperOptions;

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

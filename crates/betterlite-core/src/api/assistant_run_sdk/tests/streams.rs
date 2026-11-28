#![cfg(test)]

use rstest::rstest;
use tokio::time::timeout;

use super::*;

#[tokio::test]
async fn run_thread_stream_builds_the_manager_without_a_request() {
    let (recorder, fixture) = openai_recorder();
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let stream = sdk.run_thread_stream(call("openai")).unwrap();
    assert_eq!(stream.kind, ClientKind::Sync);
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn openai_stream_posts_stream_true_with_the_assistants_header() {
    let (recorder, fixture) = openai_recorder();
    recorder.prepare(completed_event_frames());
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    input.params.api_base = Some(String::from(BASE));
    let names = event_names(sdk.run_thread_stream(input).unwrap()).await;
    assert_eq!(
        names,
        vec![
            "thread.run.created",
            "thread.message.delta",
            "thread.run.completed"
        ]
    );
    let seen = recorder.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_a/runs"));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
    assert_eq!(seen[0].body.clone().unwrap()["stream"], true);
}

#[tokio::test]
async fn stream_events_keep_their_order_across_split_reads() {
    let (recorder, fixture) = openai_recorder();
    let all = joined(completed_event_frames());
    let split = all.len().div_ceil(2);
    recorder.prepare(vec![all[..split].to_vec(), all[split..].to_vec()]);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let names = event_names(sdk.run_thread_stream(call("openai")).unwrap()).await;
    assert_eq!(
        names,
        vec![
            "thread.run.created",
            "thread.message.delta",
            "thread.run.completed"
        ]
    );
}

#[tokio::test]
async fn openai_stream_invokes_the_supplied_handler() {
    let (recorder, fixture) = openai_recorder();
    recorder.prepare(completed_event_frames());
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let sink = recording_handler();
    let mut input = call("openai");
    input.event_handler = Some(Arc::<RecordingHandler>::clone(&sink));
    let names = event_names(sdk.run_thread_stream(input).unwrap()).await;
    assert_eq!(sink.events(), names);
    assert!(sink.ended());
}

#[tokio::test]
async fn azure_stream_drops_the_supplied_handler() {
    let (recorder, fixture) = openai_recorder();
    recorder.prepare(completed_event_frames());
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let sink = recording_handler();
    let mut params = run_params();
    params.api_base = Some(String::from(BASE));
    params.api_key = Some(secret("az-key"));
    let mut input = azure_call(params);
    input.event_handler = Some(Arc::<RecordingHandler>::clone(&sink));
    let stream = sdk.run_thread_stream(input).unwrap();
    assert!(stream.handler.is_none());
    let names = event_names(stream).await;
    assert_eq!(names.len(), 3);
    assert!(sink.events().is_empty());
    assert!(!sink.ended());
}

#[tokio::test]
async fn arun_thread_stream_returns_an_async_manager_synchronously() {
    let (recorder, fixture) = openai_recorder();
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let stream = sdk.arun_thread_stream(call("openai")).unwrap();
    assert_eq!(stream.kind, ClientKind::Asynchronous);
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn arun_thread_with_stream_true_returns_a_manager() {
    let (recorder, fixture) = openai_recorder();
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    input.stream = Some(Some(RunFlag::Bool(true)));
    let stream = manager(sdk.arun_thread(input).await.unwrap());
    assert_eq!(stream.kind, ClientKind::Asynchronous);
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn a_supplied_client_class_decides_the_manager_kind() {
    let (recorder, fixture) = openai_recorder();
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    input.client = Some(RunClient {
        class: RunClientClass::OpenAi,
        api_key: Some(secret("client-key")),
        base_url: None,
        api_version: None,
        azure_ad_token: None,
        timeout: None,
        max_retries: None,
        organization: None,
    });
    let stream = sdk.arun_thread_stream(input).unwrap();
    assert_eq!(stream.kind, ClientKind::Sync);
}

#[tokio::test]
async fn stream_entry_failure_surfaces_on_first_read() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue(404, NOT_FOUND);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let stream = sdk.run_thread_stream(call("openai")).unwrap();
    let error = stream.events().await.err().unwrap();
    assert!(matches!(error, ThreadRunsError::Status(404, _)));
}

#[tokio::test]
async fn dropping_the_manager_releases_the_transport() {
    let (recorder, fixture) = openai_recorder();
    let weak = Arc::downgrade(&recorder);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    drop(recorder);
    let stream = sdk.run_thread_stream(call("openai")).unwrap();
    assert!(weak.upgrade().is_some());
    drop(sdk);
    assert!(weak.upgrade().is_some());
    drop(stream);
    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn cancelling_the_entry_releases_the_transport() {
    let (recorder, fixture) = openai_recorder();
    let weak = Arc::downgrade(&recorder);
    let sender = recorder.gate();
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    drop(recorder);
    let stream = sdk.run_thread_stream(call("openai")).unwrap();
    let mut events = stream.events().await.unwrap();
    let cancelled = timeout(Duration::from_millis(10), events.next()).await;
    cancelled.unwrap_err();
    drop(events);
    drop(sdk);
    assert!(weak.upgrade().is_none());
    drop(sender);
}

#[rstest]
#[case::true_value(StreamArg::Bool(true))]
#[case::null(StreamArg::Null)]
#[tokio::test]
async fn run_thread_stream_rejects_a_present_stream(#[case] value: StreamArg) {
    let (recorder, fixture) = openai_recorder();
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    value.apply(&mut input);
    let error = sdk.run_thread_stream(input).err().unwrap();
    assert!(matches!(error, AssistantRunError::DuplicateStream));
    assert_eq!(
        error.to_string(),
        "litellm.assistants.main.run_thread() got multiple values for keyword argument 'stream'"
    );
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn arun_thread_stream_forces_the_async_flag() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut input = call("openai");
    input.arun_thread = Some(RunFlag::Bool(false));
    sdk.arun_thread_stream(input).unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::OpenAi { kind, handler, .. } => {
            assert_eq!(kind, ClientKind::Asynchronous);
            assert!(!handler);
        }
        CapturedTarget::Azure { .. } => panic!("the openai provider built an azure target"),
    }
}

use std::sync::Arc;
use std::time::Duration;

use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_pyhost::blocking_work::{Stats, WorkTracker};
use betterlite_pyhost::host::PythonHost;
use pyo3::types::PyAnyMethods as _;

use super::{
    PYTHON, RecordingLogger, Setup, audio_host, call, configure, decimal, probe, reset, stage,
};
use crate::api::riva::call::error::RivaCallError;
use crate::api::riva::call::transcribe;

const FINAL_SPEC: &str = r#"[{"results": [
    {"is_final": true, "alternatives": [{"transcript": "hello "}]},
    {"is_final": true, "alternatives": [{"transcript": "world"}]}
]}]"#;

async fn wait_for_event(host: &PythonHost, event: &str) -> bool {
    for _attempt in 0..500 {
        let recorded = probe(host);
        if recorded.events.iter().any(|name| name == event) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

async fn wait_for_jobs(tracker: &WorkTracker) -> bool {
    for _attempt in 0..500 {
        let stats = tracker.stats();
        if stats.running == 0 && stats.completed == 3 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

fn release_gate(host: &PythonHost) {
    host.attach(|py| {
        let gate = py
            .import("riva_call_probe")
            .unwrap()
            .getattr("gate")
            .unwrap();
        gate.call_method0("set").unwrap();
    });
}

fn spawn_call(
    host: PythonHost,
    tracker: Arc<WorkTracker>,
    optional: String,
    api_base: String,
) -> tokio::task::JoinHandle<Result<TranscriptionReply, RivaCallError>> {
    tokio::spawn(async move {
        let setup = Setup::new(&optional);
        let pending = call(&setup, Some(api_base.as_str()), false);
        let mut logger = RecordingLogger::default();
        transcribe(
            &host,
            &tracker,
            Duration::from_secs(30),
            &pending,
            &mut logger,
        )
        .await
    })
}

#[tokio::test]
async fn async_entry_runs_the_same_pipeline_and_logs_atranscription() {
    let _guard = PYTHON.lock().await;
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new("{}");
    let tracker = WorkTracker::new(2, Duration::from_secs(5));
    let mut logger = RecordingLogger::default();
    let pending = call(&setup, Some("grpc.example:443"), false);
    let reply = transcribe(
        &host,
        &tracker,
        Duration::from_secs(30),
        &pending,
        &mut logger,
    )
    .await
    .unwrap();
    assert_eq!(reply.response.text.as_deref(), Some("hello world"));
    assert_eq!(reply.duration, StoredDuration::Known(Some(decimal("0.1"))));
    assert_eq!(logger.pre.len(), 1);
    assert!(logger.pre[0].atranscription);
    let recorded = probe(&host);
    assert_eq!(recorded.chunk_sizes, vec![vec![3200_usize]]);
    assert_eq!(recorded.seen_languages, vec!["en-US"]);
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 0,
            abandoned: 0,
            completed: 3
        }
    );
}

#[tokio::test]
async fn cancelling_the_call_leaves_no_tracked_job() {
    let _guard = PYTHON.lock().await;
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    configure(&host, &["block_stream"]);
    let tracker = Arc::new(WorkTracker::new(2, Duration::from_secs(5)));
    let handle = spawn_call(
        host.clone(),
        Arc::clone(&tracker),
        String::from("{}"),
        String::from("grpc.example:443"),
    );
    assert!(wait_for_event(&host, "stream.blocked").await);
    handle.abort();
    let joined = handle.await;
    assert!(joined.is_err());
    release_gate(&host);
    assert!(wait_for_jobs(&tracker).await, "{:?}", tracker.stats());
    let stats = tracker.stats();
    assert_eq!(stats.running, 0);
    assert_eq!(stats.abandoned, 0);
    assert_eq!(stats.completed, 3);
}

#[tokio::test]
async fn concurrent_calls_keep_their_own_sdk_objects() {
    let _guard = PYTHON.lock().await;
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    let tracker = Arc::new(WorkTracker::new(2, Duration::from_secs(5)));
    let first = spawn_call(
        host.clone(),
        Arc::clone(&tracker),
        String::from(r#"{"language_code":"en-US","nvcf_function_id":"fn-a"}"#),
        String::from("grpc.nvcf.nvidia.com:443"),
    );
    let second = spawn_call(
        host.clone(),
        Arc::clone(&tracker),
        String::from(r#"{"language_code":"de-DE"}"#),
        String::from("localhost:50051"),
    );
    let (first_outcome, second_outcome) = tokio::join!(first, second);
    first_outcome.unwrap().unwrap();
    second_outcome.unwrap().unwrap();
    let recorded = probe(&host);
    assert_eq!(recorded.auth_ids.len(), 2);
    assert_eq!(recorded.auth_used_ids.len(), 2);
    assert!(
        recorded
            .auth_ids
            .iter()
            .all(|id| recorded.auth_used_ids.contains(id))
    );
    assert_ne!(recorded.auth_ids[0], recorded.auth_ids[1]);
    assert_eq!(recorded.recognition_ids.len(), 2);
    assert_ne!(recorded.recognition_ids[0], recorded.recognition_ids[1]);
    assert_eq!(recorded.generator_config_ids.len(), 2);
    assert!(
        recorded
            .generator_config_ids
            .iter()
            .all(|id| recorded.recognition_ids.contains(id))
    );
    let mut languages = recorded.seen_languages;
    languages.sort();
    assert_eq!(languages, vec!["de-DE", "en-US"]);
}

#![cfg(test)]

use std::env;
use std::fs::{self, File};
use std::process;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use betterlite_lens::models::execution::Coverage;
use tokio::task::JoinSet;
use tokio::time::{Instant, advance, sleep};
use tracing::subscriber::set_default;

use super::fixtures::{FakeApi, claim};
use crate::api::WorkerError;
use crate::worker::LensWorker;

fn status(path: &str, status: u16) -> WorkerError {
    WorkerError::Status {
        path: path.to_owned(),
        status,
    }
}

#[tokio::test]
async fn returns_false_when_no_job_is_claimed() {
    let worker = LensWorker {
        api: Arc::new(FakeApi::default()),
    };
    assert!(!worker.run_once().await.unwrap());
    assert!(worker.api.results.lock().unwrap().is_empty());
    assert!(worker.api.heartbeats.lock().unwrap().is_empty());
}

#[tokio::test]
async fn posts_the_analysis_result_after_a_successful_run() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    let results = worker.api.results.lock().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].error, "");
    assert_eq!(results[0].coverage.eligible, 1);
    assert_eq!(results[0].coverage.screened, 1);
    assert_eq!(results[0].findings.len(), 1);
    assert_eq!(results[0].findings[0].evidence[0].execution_id, "exec");
    assert_eq!(results[0].assessments.len(), 1);
    assert_eq!(results[0].assessments[0].execution_id, "exec");
    drop(results);
    assert_eq!(
        *worker.api.content_calls.lock().unwrap(),
        vec![(String::from("exec"), String::new(), 0)]
    );
    assert!(!worker.api.progress_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn posts_the_failure_text_when_the_sample_fetch_fails() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.samples
        .lock()
        .unwrap()
        .push_back(Err(status("/lens/worker/lens/job/sample", 404)));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    let results = worker.api.results.lock().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].coverage, Coverage::default());
    assert!(results[0].findings.is_empty());
    assert_eq!(
        results[0].error,
        "Reading trace data failed (HTTP 404). Check that the proxy and worker versions match and the requested model is configured."
    );
}

#[tokio::test]
async fn posts_the_callback_text_when_a_model_request_fails() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Err(status("/lens/worker/lens/job/model", 400)));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    let results = worker.api.results.lock().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].error,
        "Model request failed (HTTP 400). Check the configured model and whether the worker's billing key is enabled."
    );
}

#[tokio::test]
async fn posts_the_failure_text_when_reading_content_fails() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.pages
        .lock()
        .unwrap()
        .push_back(Err(status("/lens/worker/lens/job/content", 403)));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    let results = worker.api.results.lock().unwrap();
    assert_eq!(
        results[0].error,
        "Reading trace data failed (HTTP 403). Check the worker key's model permissions and access restrictions."
    );
}

#[tokio::test]
async fn posts_the_error_result_when_the_success_post_fails() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.result_answers
        .lock()
        .unwrap()
        .push_back(Err(status("/lens/worker/lens/job/result", 503)));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    let results = worker.api.results.lock().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].error, "");
    assert_eq!(results[1].coverage, Coverage::default());
    assert_eq!(
        results[1].error,
        "Saving results failed (HTTP 503). Check proxy and model availability, then retry the investigation."
    );
}

#[tokio::test]
async fn accepts_a_conflict_answer_when_posting_the_failed_result() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.samples
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Timeout));
    fake.result_answers
        .lock()
        .unwrap()
        .push_back(Err(status("/lens/worker/lens/job/result", 409)));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    assert_eq!(worker.api.results.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn returns_another_failed_status_from_the_failure_post() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.samples
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Timeout));
    fake.result_answers
        .lock()
        .unwrap()
        .push_back(Err(status("/lens/worker/lens/job/result", 500)));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    let error = worker.run_once().await.unwrap_err();
    assert!(matches!(error, WorkerError::Status { status: 500, .. }));
    assert_eq!(worker.api.results.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn sends_a_heartbeat_every_thirty_seconds_while_the_job_runs() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    *fake.sample_delay.lock().unwrap() = Duration::from_secs(65);
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    let started = Instant::now();
    assert!(worker.run_once().await.unwrap());
    let beats = worker.api.heartbeats.lock().unwrap();
    assert_eq!(beats.len(), 2);
    assert_eq!(beats[0].duration_since(started), Duration::from_secs(30));
    assert_eq!(beats[1].duration_since(started), Duration::from_secs(60));
}

#[tokio::test(start_paused = true)]
async fn stops_heartbeating_after_the_first_failure() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    *fake.heartbeat_failure.lock().unwrap() = Some(WorkerError::Transport);
    *fake.sample_delay.lock().unwrap() = Duration::from_secs(100);
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    assert_eq!(worker.api.heartbeats.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn stops_the_heartbeat_when_the_run_returns() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.samples
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Timeout));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    assert!(worker.api.heartbeats.lock().unwrap().is_empty());
    advance(Duration::from_secs(90)).await;
    sleep(Duration::from_millis(1)).await;
    assert!(worker.api.heartbeats.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn pauses_ten_seconds_after_every_pass() {
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.samples
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Timeout));
    // the failure post fails too, so the first pass returns an error and the loop warns and goes on
    fake.result_answers
        .lock()
        .unwrap()
        .push_back(Err(status("/lens/worker/lens/job/result", 500)));
    let worker = Arc::new(LensWorker {
        api: Arc::new(fake),
    });
    let mut runs = JoinSet::new();
    let runner = Arc::clone(&worker);
    runs.spawn(async move {
        runner.run_forever().await;
    });
    sleep(Duration::from_secs(15)).await;
    runs.abort_all();
    while runs.join_next().await.is_some() {}
    let claims = worker.api.claim_calls.lock().unwrap();
    assert_eq!(claims.len(), 2);
    assert_eq!(claims[1].duration_since(claims[0]), Duration::from_secs(10));
}

#[tokio::test]
async fn warns_with_the_error_kind_but_never_its_text() {
    let name = thread::current().name().unwrap().replace("::", "-");
    let log_path = env::temp_dir().join(format!(
        "betterlite-lens-worker-{}-{name}.log",
        process::id()
    ));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(File::create(&log_path).unwrap()))
        .finish();
    let _guard = set_default(subscriber);
    let fake = FakeApi::default();
    fake.claimed.lock().unwrap().push_back(Ok(Some(claim())));
    fake.samples
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Decode(String::from(
            "https://proxy.example/secret",
        ))));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    assert!(worker.run_once().await.unwrap());
    let written = fs::read_to_string(&log_path).unwrap();
    fs::remove_file(&log_path).unwrap();
    assert!(written.contains("Analysis job interrupted (Decode)"));
    assert!(!written.contains("https://proxy.example/secret"));
}

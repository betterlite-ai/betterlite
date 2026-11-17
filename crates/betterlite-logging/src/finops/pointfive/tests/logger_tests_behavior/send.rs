use std::io::Read as _;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use flate2::read::GzDecoder;

use super::support::{
    FakeHttp, PRESIGNED, RecordingSleeper, Reply, client, client_with, core, core_with,
    default_params, logged, payload, target_body,
};
use crate::finops::pointfive::logger::flush::flush_queue;
use crate::finops::pointfive::logger::send::send_batch;

fn upload_pair(index: usize) -> [Reply; 2] {
    [
        Reply::with_body(
            200,
            &target_body(&format!("{PRESIGNED}/{index}"), &format!("objects/{index}")),
        ),
        Reply::answer(200),
    ]
}

fn params(max_batch_bytes: usize) -> PointFiveInitParams {
    let mut params = default_params();
    params.max_batch_bytes = max_batch_bytes;
    params
}

fn decompressed(body: &[u8]) -> String {
    let mut decoder = GzDecoder::new(body);
    let mut text = String::new();
    decoder
        .read_to_string(&mut text)
        .expect("the upload is gzipped ndjson");
    text
}

#[tokio::test]
async fn an_empty_queue_sends_nothing() {
    let http = FakeHttp::default();
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    send_batch(&core).await.expect("an empty batch is fine");
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn one_chunk_ships_every_record() {
    let pair = upload_pair(0);
    let http = FakeHttp::scripted(vec![pair[0].clone(), pair[1].clone()]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    core.queue.push(payload("evt-1"));
    core.queue.push(payload("evt-2"));
    flush_queue(&core, false).await;
    let uploads = http.uploads();
    assert_eq!(uploads.len(), 1);
    let text = decompressed(&uploads[0].body);
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains("evt-1"));
    assert!(text.contains("evt-2"));
    assert!(core.queue.is_empty());
}

#[tokio::test]
async fn a_batch_over_the_byte_limit_is_split_into_objects() {
    let first = upload_pair(0);
    let second = upload_pair(1);
    let http = FakeHttp::scripted(vec![
        first[0].clone(),
        first[1].clone(),
        second[0].clone(),
        second[1].clone(),
    ]);
    let core = core(params(1), client(&http, 1));
    core.queue.push(payload("evt-1"));
    core.queue.push(payload("evt-2"));
    flush_queue(&core, false).await;
    let uploads = http.uploads();
    assert_eq!(uploads.len(), 2);
    assert_eq!(decompressed(&uploads[0].body).lines().count(), 1);
    assert!(decompressed(&uploads[1].body).contains("evt-2"));
    assert!(core.queue.is_empty());
}

#[tokio::test]
async fn a_retryable_failure_raises_and_keeps_the_failed_chunk() {
    let first = upload_pair(0);
    let http = FakeHttp::scripted(vec![first[0].clone(), first[1].clone(), Reply::answer(500)]);
    let core = core(params(1), client(&http, 1));
    core.queue.push(payload("evt-1"));
    core.queue.push(payload("evt-2"));
    let error = send_batch(&core)
        .await
        .expect_err("the retryable failure is raised");
    assert!(error.to_string().contains("pointfive api returned 500"));
    assert!(error.to_string().contains("gave up after 1 attempts"));
    let held: Vec<String> = core
        .queue
        .snapshot()
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(held, vec!["evt-2".to_owned()]);
}

#[tokio::test]
async fn a_rejected_chunk_is_logged_and_dropped() {
    let http = FakeHttp::scripted(vec![Reply::answer(400)]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    core.queue.push(payload("evt-1"));
    let ((), text) = logged(flush_queue(&core, false)).await;
    assert!(core.queue.is_empty());
    assert!(text.contains("dropping records"));
    assert!(text.contains("pointfive api returned 400"));
}

#[tokio::test]
async fn a_retryable_failure_waits_a_backoff_and_then_succeeds() {
    let pair = upload_pair(0);
    let http = FakeHttp::scripted(vec![Reply::answer(500), pair[0].clone(), pair[1].clone()]);
    let sleeper = RecordingSleeper::default();
    let upload_client = client_with(&http, 2, sleeper.clone());
    let core = core_with(PointFiveInitParams::default(), upload_client, false);
    core.queue.push(payload("evt-1"));
    flush_queue(&core, false).await;
    assert_eq!(http.uploads().len(), 1);
    let delays = sleeper
        .delays
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(delays, vec![Duration::from_secs(1)]);
    assert!(core.queue.is_empty());
}

#[tokio::test]
async fn a_failure_past_the_retry_limit_raises() {
    let http = FakeHttp::scripted(vec![Reply::answer(500), Reply::answer(500)]);
    let core = core(PointFiveInitParams::default(), client(&http, 2));
    core.queue.push(payload("evt-1"));
    let error = send_batch(&core).await.expect_err("the failure is raised");
    assert!(error.to_string().contains("gave up after 2 attempts"));
    assert_eq!(core.queue.len(), 1);
}

#[tokio::test]
async fn a_transport_failure_is_retryable() {
    let http = FakeHttp::scripted(vec![Reply::Transport]);
    let core = core(PointFiveInitParams::default(), client(&http, 1));
    core.queue.push(payload("evt-1"));
    let error = send_batch(&core)
        .await
        .expect_err("the transport failure is raised");
    assert!(error.to_string().contains("pointfive api unreachable"));
    assert_eq!(core.queue.len(), 1);
}

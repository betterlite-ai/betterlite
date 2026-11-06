use futures_util::StreamExt as _;

use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::operations::RunInvocation;

use super::cases::{Facade, call, frame, request};
use super::support::{IN_PROGRESS, harness, parts_chunks};

const CHUNKS: usize = 48;
const FRAMES_PER_CHUNK: usize = 100;

// Every frame stays far below the 1 MiB frame cap while the stream's total runs past the 16 MiB event cap.
fn fat_frame() -> String {
    let fat = IN_PROGRESS.replace(
        "\"instructions\":\"base\"",
        &format!("\"instructions\":\"{}\"", "y".repeat(4096)),
    );
    frame("thread.run.in_progress", &fat)
}

fn chunks() -> Vec<Result<Vec<u8>, ThreadRunsError>> {
    let chunk = fat_frame();
    let part = chunk.repeat(FRAMES_PER_CHUNK);
    let mut chunks = Vec::new();
    for _chunk in 0..CHUNKS {
        chunks.push(Ok(part.as_bytes().to_vec()));
    }
    chunks
}

// The frame buffer is capped per frame, so one oversized frame fails while a long stream of small ones completes.
pub(super) async fn stream_cap_is_bounded() {
    long_stream_completes().await;
    oversized_frame_fails().await;
    consume_cap_is_bounded().await;
}

async fn long_stream_completes() {
    let (api, transport) = harness();
    transport.stream_chunks(parts_chunks(chunks()));
    let outcome = api
        .run_thread(call(Facade::SyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let events: Vec<Result<_, ThreadRunsError>> = stream.events().await.unwrap().collect().await;
    assert_eq!(events.len(), 4_800);
    for item in events {
        assert_eq!(item.unwrap().name(), "thread.run.in_progress");
    }
}

async fn oversized_frame_fails() {
    let (api, transport) = harness();
    let oversized = format!("data: {}\n\n", "x".repeat(0x0010_0000));
    transport.stream_chunks(parts_chunks(vec![Ok(oversized.into_bytes())]));
    let outcome = api
        .run_thread(call(Facade::SyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let mut events = stream.events().await.unwrap();
    let failure = events.next().await.expect("a failure").err().unwrap();
    assert!(
        matches!(failure, ThreadRunsError::StreamTooLarge),
        "{failure}"
    );
}

// The consumer retains every event it collects, so its own total cap stops a stream the frame cap allows.
async fn consume_cap_is_bounded() {
    let (api, transport) = harness();
    transport.stream_chunks(parts_chunks(chunks()));
    let outcome = api
        .run_thread(call(Facade::SyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let failure = stream.consume().await.err().unwrap();
    assert!(
        matches!(failure, ThreadRunsError::StreamTooLarge),
        "{failure}"
    );
}

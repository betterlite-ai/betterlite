use futures_util::StreamExt as _;

use crate::thread_runs::decode::{FrameOutcome, decode_run_event};
use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::operations::RunInvocation;
use crate::thread_runs::stream::MAX_EVENT_BYTES;

use super::cases::{Facade, call, frame, request};
use super::support::{IN_PROGRESS, harness, parts_chunks};

const CHUNKS: usize = 48;
const FRAMES_PER_CHUNK: usize = 100;
const PADDING: usize = 0x1000;

// Every frame stays far below the 1 MiB frame cap while the stream's total runs past the 16 MiB event cap.
fn fat_frame(padding: usize) -> String {
    let fat = IN_PROGRESS.replace(
        "\"instructions\":\"base\"",
        &format!("\"instructions\":\"{}\"", "y".repeat(padding)),
    );
    frame("thread.run.in_progress", &fat)
}

fn chunks() -> Vec<Result<Vec<u8>, ThreadRunsError>> {
    let chunk = fat_frame(PADDING);
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
    frame_cap_boundary().await;
    consume_cap_is_bounded().await;
    consume_boundary_is_allowed().await;
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

// The frame buffer admits exactly 1 MiB, then the next byte fails the cap instead of the JSON parse.
async fn frame_cap_boundary() {
    for (padding, over_cap) in [(0x000F_FFF8_usize, false), (0x0010_0000_usize, true)] {
        let chunk = format!("data: {}\n\n", "x".repeat(padding));
        let (api, transport) = harness();
        transport.stream_chunks(parts_chunks(vec![Ok(chunk.into_bytes())]));
        let outcome = api
            .run_thread(call(Facade::SyncStream), &request())
            .await
            .unwrap();
        let RunInvocation::Streaming(stream) = outcome else {
            panic!("expected a stream manager");
        };
        let mut events = stream.events().await.unwrap();
        let failure = events.next().await.expect("a failure").err().unwrap();
        if over_cap {
            assert!(
                matches!(failure, ThreadRunsError::StreamTooLarge),
                "{failure}"
            );
        } else {
            assert!(
                matches!(failure, ThreadRunsError::Malformed(_)),
                "{failure}"
            );
        }
    }
}

// Exactly the retained cap is admitted, so the last event of 16 MiB is kept.
async fn consume_boundary_is_allowed() {
    let probe = fat_frame(PADDING);
    let Ok(FrameOutcome::Event(event)) = decode_run_event(probe.as_bytes()) else {
        panic!("the probe frame decodes to one event");
    };
    let size = serde_json::to_vec(&event).unwrap().len();
    let target = size.next_power_of_two();
    let padding = PADDING.saturating_add(target.saturating_sub(size));
    let count = MAX_EVENT_BYTES.checked_div(target).unwrap();
    assert_eq!(count.checked_rem(64), Some(0));
    let part = fat_frame(padding).repeat(count.checked_div(64).unwrap());
    let mut parts = Vec::new();
    for _chunk in 0..64_usize {
        parts.push(Ok(part.as_bytes().to_vec()));
    }
    let (api, transport) = harness();
    transport.stream_chunks(parts_chunks(parts));
    let outcome = api
        .run_thread(call(Facade::SyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let events = stream.consume().await.unwrap();
    assert_eq!(events.len(), count);
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

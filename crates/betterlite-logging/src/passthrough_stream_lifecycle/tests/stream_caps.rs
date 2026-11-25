#![cfg(test)]

use indexmap::IndexMap;

use crate::passthrough_stream_lifecycle::chunk_processor::{
    ChunkProcessorCall, CompletionGuard, MAX_LOGGED_STREAM_BYTES, MAX_PENDING_FRAME_BYTES,
    StreamBufferCaps,
};
use crate::passthrough_stream_lifecycle::support::{
    Harness, capture_logs, json, object, processor_call,
};
use crate::passthrough_stream_logging::types::EndpointType;

fn plain_call(harness: &Harness, endpoint_type: EndpointType) -> ChunkProcessorCall {
    processor_call(
        harness.lifecycle(false),
        object(&[], IndexMap::new()),
        endpoint_type,
    )
}

fn injection_call(harness: &Harness) -> ChunkProcessorCall {
    let mut call = processor_call(
        harness.lifecycle(true),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.request_body = Some(json("{\"model\":\"gpt-4o\"}"));
    call
}

async fn the_logged_chunks(harness: &Harness) -> Vec<Vec<u8>> {
    let jobs = harness.queue.take_all();
    assert_eq!(jobs.len(), 1);
    for job in jobs {
        job.work.await;
    }
    let routes = harness.route.calls();
    assert_eq!(routes.len(), 1);
    routes[0].raw_bytes.clone()
}

#[test]
fn the_defaults_cap_the_log_copy_at_64_mib_and_an_incomplete_frame_at_16_mib() {
    assert_eq!(MAX_LOGGED_STREAM_BYTES, 0x0400_0000);
    assert_eq!(MAX_PENDING_FRAME_BYTES, 0x0100_0000);
    assert_eq!(StreamBufferCaps::DEFAULT.logged_bytes, 0x0400_0000);
    assert_eq!(StreamBufferCaps::DEFAULT.frame_bytes, 0x0100_0000);
}

#[tokio::test]
async fn a_chunk_at_the_log_cap_is_kept_and_the_one_past_it_is_not() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let (logs, _logs_guard) = capture_logs();
    let caps = StreamBufferCaps {
        logged_bytes: 8,
        frame_bytes: MAX_PENDING_FRAME_BYTES,
    };
    let mut run = CompletionGuard::new(plain_call(&harness, EndpointType::OpenAi), caps);
    let fits = b"12345678".to_vec();
    assert_eq!(run.accept(fits.clone()), Some(fits.clone()));
    assert_eq!(run.accept(b"9".to_vec()), Some(b"9".to_vec()));
    drop(run);
    assert_eq!(the_logged_chunks(&harness).await, vec![fits]);
    assert_eq!(logs.text().matches("its log copy stops here").count(), 1);
}

#[tokio::test]
async fn a_stream_past_the_log_cap_forwards_every_chunk_and_logs_only_the_kept_prefix() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let (logs, _logs_guard) = capture_logs();
    let caps = StreamBufferCaps {
        logged_bytes: 8,
        frame_bytes: MAX_PENDING_FRAME_BYTES,
    };
    let mut run = CompletionGuard::new(plain_call(&harness, EndpointType::OpenAi), caps);
    let chunks = vec![b"aaaaa".to_vec(), b"bbbbb".to_vec(), b"ccccc".to_vec()];
    let mut yielded = Vec::new();
    for chunk in chunks.clone() {
        yielded.extend(run.accept(chunk));
    }
    drop(run);
    assert_eq!(yielded, chunks);
    assert_eq!(the_logged_chunks(&harness).await, vec![b"aaaaa".to_vec()]);
    assert_eq!(logs.text().matches("its log copy stops here").count(), 1);
}

#[test]
fn an_incomplete_injection_frame_at_the_cap_still_waits_for_its_end() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let (logs, _logs_guard) = capture_logs();
    let caps = StreamBufferCaps {
        logged_bytes: MAX_LOGGED_STREAM_BYTES,
        frame_bytes: 16,
    };
    let mut run = CompletionGuard::new(injection_call(&harness), caps);
    let mut frame = vec![b'a'; 16];
    assert_eq!(run.accept(frame.clone()), None);
    frame.extend_from_slice(b"\n\n");
    let mut expected = b"injected:".to_vec();
    expected.extend_from_slice(&frame);
    assert_eq!(run.accept(b"\n\n".to_vec()), Some(expected));
    assert_eq!(
        harness.cost_injection.calls(),
        vec![(frame, "gpt-4o".to_owned())]
    );
    assert!(!logs.text().contains("cost-injection frame is past"));
}

#[tokio::test]
async fn an_incomplete_injection_frame_past_the_cap_is_forwarded_and_injection_stops() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let (logs, _logs_guard) = capture_logs();
    let caps = StreamBufferCaps {
        logged_bytes: MAX_LOGGED_STREAM_BYTES,
        frame_bytes: 16,
    };
    let mut run = CompletionGuard::new(injection_call(&harness), caps);
    let first = vec![b'a'; 10];
    let second = vec![b'b'; 10];
    let later = b"data: x\n\n".to_vec();
    let mut buffered = first.clone();
    buffered.extend_from_slice(&second);
    assert_eq!(run.accept(first.clone()), None);
    assert_eq!(run.accept(second.clone()), Some(buffered));
    assert_eq!(run.accept(later.clone()), Some(later.clone()));
    drop(run);
    assert!(harness.cost_injection.calls().is_empty());
    assert_eq!(
        logs.text().matches("cost-injection frame is past").count(),
        1
    );
    assert_eq!(
        the_logged_chunks(&harness).await,
        vec![first, second, later]
    );
}

#[test]
fn complete_frames_before_an_over_cap_tail_are_still_injected() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let (logs, _logs_guard) = capture_logs();
    let caps = StreamBufferCaps {
        logged_bytes: MAX_LOGGED_STREAM_BYTES,
        frame_bytes: 16,
    };
    let mut run = CompletionGuard::new(injection_call(&harness), caps);
    let tail = vec![b'z'; 20];
    let mut chunk = b"data: a\n\n".to_vec();
    chunk.extend_from_slice(&tail);
    let mut expected = b"injected:data: a\n\n".to_vec();
    expected.extend_from_slice(&tail);
    assert_eq!(run.accept(chunk), Some(expected));
    assert_eq!(
        harness.cost_injection.calls(),
        vec![(b"data: a\n\n".to_vec(), "gpt-4o".to_owned())]
    );
    let later = b"data: b\n\n".to_vec();
    let unchanged = later.clone();
    assert_eq!(run.accept(later), Some(unchanged));
    assert_eq!(
        logs.text().matches("cost-injection frame is past").count(),
        1
    );
}

#[tokio::test]
async fn a_tinyfish_frame_past_the_cap_stops_the_scan_and_forwards_every_chunk() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_tinyfish(false, Some("run-1"));
    let (logs, _logs_guard) = capture_logs();
    let caps = StreamBufferCaps {
        logged_bytes: MAX_LOGGED_STREAM_BYTES,
        frame_bytes: 16,
    };
    let mut run = CompletionGuard::new(plain_call(&harness, EndpointType::Tinyfish), caps);
    let first = vec![b'a'; 10];
    let second = vec![b'b'; 10];
    let frame = b"data: {\"run_id\":\"run-1\"}\n\n".to_vec();
    assert_eq!(run.accept(first.clone()), Some(first.clone()));
    assert_eq!(run.accept(second.clone()), Some(second.clone()));
    assert_eq!(run.accept(frame.clone()), Some(frame.clone()));
    drop(run);
    assert!(harness.tinyfish.scans.lock().unwrap().is_empty());
    assert!(harness.tinyfish.started().is_empty());
    assert_eq!(
        logs.text().matches("TinyFish run_id scan is past").count(),
        1
    );
    assert_eq!(
        the_logged_chunks(&harness).await,
        vec![first, second, frame]
    );
}

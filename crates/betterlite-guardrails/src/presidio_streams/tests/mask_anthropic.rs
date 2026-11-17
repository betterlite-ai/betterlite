#![cfg(test)]

use super::shared::{DELTA_FRAME, Harness, Kind, START_FRAME, bytes, ok, request_data};

#[tokio::test]
async fn assembles_the_first_frame_and_every_later_chunk_then_masks() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![
        bytes(START_FRAME),
        bytes(DELTA_FRAME),
        bytes(DELTA_FRAME),
    ]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent, ["bytes rebuilt:MASKED"]);
    let seen = harness.assembly.seen_frames.lock().unwrap().clone();
    assert_eq!(
        seen,
        [vec![
            START_FRAME.as_bytes().to_vec(),
            DELTA_FRAME.as_bytes().to_vec(),
            DELTA_FRAME.as_bytes().to_vec(),
        ]]
    );
}

#[tokio::test]
async fn the_leading_frames_are_not_part_of_the_assembly() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![bytes(": ka\n\n"), bytes(START_FRAME)]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent, ["bytes : ka\n\n", "bytes rebuilt:MASKED"]);
    let seen = harness.assembly.seen_frames.lock().unwrap().clone();
    assert_eq!(seen, [vec![START_FRAME.as_bytes().to_vec()]]);
}

#[tokio::test]
async fn masking_records_the_guardrail_information_on_the_request() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![bytes(START_FRAME)]);
    let (_sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    let metadata = request.get("metadata").unwrap().get();
    assert!(metadata.contains("standard_logging_guardrail_information"));
}

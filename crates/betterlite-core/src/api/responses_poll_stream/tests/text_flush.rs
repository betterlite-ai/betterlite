use std::sync::atomic::Ordering;

use super::frame_selection;
use super::support::{self, Answer, AnswerStep, Scenario};

const ITEM_ADDED: &str =
    r#"{"item":{"content":[],"id":"m","type":"message"},"type":"response.output_item.added"}"#;
const PART_ADDED: &str = r#"{"content_index":0,"item_id":"m","part":{"text":"","type":"output_text"},"type":"response.content_part.added"}"#;
const HE: &str =
    r#"{"content_index":0,"delta":"he","item_id":"m","type":"response.output_text.delta"}"#;
const LLO: &str =
    r#"{"content_index":0,"delta":"llo","item_id":"m","type":"response.output_text.delta"}"#;

pub(super) const CLOCK: [u64; 6] = [1000, 1149, 1150, 1200, 1299, 1400];

pub(super) async fn flushed_stream() -> Scenario {
    let mut steps: Vec<AnswerStep> = support::frames(&[ITEM_ADDED, PART_ADDED, HE, LLO])
        .into_iter()
        .map(AnswerStep::Chunk)
        .collect();
    steps.extend(
        support::frames(&["[DONE]"])
            .into_iter()
            .map(AnswerStep::Chunk),
    );
    let mut scenario = support::scenario(&CLOCK, Answer::Streaming(steps));
    scenario.run().await.expect("the worker completes");
    scenario
}

#[tokio::test]
async fn flush_stays_quiet_without_dirty_output() {
    let unknown = r#"{"type":"response.something_else"}"#;
    let chunks = support::frames(&[ITEM_ADDED, unknown]);
    let (scenario, _warnings) =
        frame_selection::run_chunks(chunks, &[1000, 1200, 1400, 1400]).await;
    let writes = scenario.cache.writes();
    assert_eq!(writes.len(), 3);
    assert_eq!(support::text(&writes[0], "status"), "in_progress");
    assert_eq!(support::text(&writes[2], "status"), "completed");
}

pub(super) fn assert_batches(scenario: &Scenario) {
    let writes = scenario.cache.writes();
    assert_eq!(writes.len(), 4);
    assert_eq!(support::text(&writes[0], "status"), "in_progress");
    assert_eq!(support::first_text(&writes[1]), "");
    assert_eq!(support::first_text(&writes[2]), "hello");
    assert_eq!(support::text(&writes[3], "status"), "completed");
    assert_eq!(support::first_text(&writes[3]), "hello");
}

pub(super) fn assert_clock_reads(scenario: &Scenario) {
    assert_eq!(scenario.clock.reads.load(Ordering::SeqCst), CLOCK.len());
}

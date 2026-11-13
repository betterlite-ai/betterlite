#![cfg(test)]

use std::num::NonZeroUsize;
use std::time::Duration;

use rstest::rstest;

use super::{WxoStreamResult, WxoStreamStep, fake_streaming_from_text};
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::pydantic_ai_transformation::fake_streaming::events::ArtifactUpdateResult;
use crate::watsonx_transform::tests::{Source, uuid};

fn pacing(chunk_size: usize, delay_ms: u64) -> StreamPacing {
    StreamPacing {
        chunk_size: NonZeroUsize::new(chunk_size).unwrap(),
        delay: Duration::from_millis(delay_ms),
    }
}

fn stream(text: &str, pacing: StreamPacing) -> Vec<WxoStreamStep> {
    fake_streaming_from_text(text, "req-1", pacing, &mut Source::default())
}

fn artifacts(steps: &[WxoStreamStep]) -> Vec<&ArtifactUpdateResult> {
    steps
        .iter()
        .filter_map(|step| match &step.event.result {
            WxoStreamResult::Artifact(result) => Some(result),
            WxoStreamResult::Task(_) | WxoStreamResult::Status(_) => None,
        })
        .collect()
}

fn artifact_texts(steps: &[WxoStreamStep]) -> Vec<String> {
    artifacts(steps)
        .iter()
        .map(|result| result.artifact.parts[0].text.clone().unwrap())
        .collect()
}

#[test]
fn streaming_sends_task_working_artifacts_then_final_completed() {
    let steps = stream("hello", StreamPacing::default());
    let json = steps
        .iter()
        .map(|step| serde_json::to_string(&step.event).unwrap())
        .collect::<Vec<_>>();
    let (task, context, artifact) = (uuid(1), uuid(2), uuid(3));
    assert_eq!(
        json,
        [
            format!(
                r#"{{"id":"req-1","jsonrpc":"2.0","result":{{"contextId":"{context}","id":"{task}","kind":"task","status":{{"state":"submitted"}}}}}}"#
            ),
            format!(
                r#"{{"id":"req-1","jsonrpc":"2.0","result":{{"contextId":"{context}","final":false,"kind":"status-update","status":{{"state":"working"}},"taskId":"{task}"}}}}"#
            ),
            format!(
                r#"{{"id":"req-1","jsonrpc":"2.0","result":{{"contextId":"{context}","kind":"artifact-update","taskId":"{task}","artifact":{{"artifactId":"{artifact}","parts":[{{"kind":"text","text":"hello"}}]}}}}}}"#
            ),
            format!(
                r#"{{"id":"req-1","jsonrpc":"2.0","result":{{"contextId":"{context}","final":true,"kind":"status-update","status":{{"state":"completed"}},"taskId":"{task}"}}}}"#
            ),
        ]
    );
}

#[test]
fn streaming_defaults_to_fifty_characters_every_ten_milliseconds() {
    let default = StreamPacing::default();
    assert_eq!(default.chunk_size.get(), 50);
    assert_eq!(default.delay, Duration::from_millis(10));
}

#[rstest]
#[case::empty(0, &[0])]
#[case::one_character(1, &[1])]
#[case::exactly_one_chunk(50, &[50])]
#[case::one_over(51, &[50, 1])]
#[case::two_full_chunks(100, &[50, 50])]
#[case::two_full_and_one(101, &[50, 50, 1])]
fn streaming_splits_text_into_fifty_character_artifacts(
    #[case] length: usize,
    #[case] expected: &[usize],
) {
    let text = "x".repeat(length);
    let chunks = artifact_texts(&stream(&text, StreamPacing::default()));
    assert_eq!(chunks.iter().map(String::len).collect::<Vec<_>>(), expected);
    assert_eq!(chunks.concat(), text);
}

#[test]
fn streaming_empty_text_still_emits_one_empty_artifact() {
    let steps = stream("", StreamPacing::default());
    assert_eq!(steps.len(), 4);
    assert_eq!(artifact_texts(&steps), [String::new()]);
}

#[test]
fn streaming_waits_after_working_and_between_artifacts_only() {
    let steps = stream("abcdefg", pacing(3, 25));
    let delays = steps
        .iter()
        .map(|step| step.delay_after)
        .collect::<Vec<_>>();
    let wait = Duration::from_millis(25);
    assert_eq!(
        delays,
        [
            Duration::ZERO,
            wait,
            wait,
            wait,
            Duration::ZERO,
            Duration::ZERO
        ]
    );
    assert_eq!(artifact_texts(&steps), ["abc", "def", "g"]);
}

#[test]
fn streaming_single_artifact_has_no_wait_after_it() {
    let steps = stream("abc", pacing(3, 25));
    let delays = steps
        .iter()
        .map(|step| step.delay_after)
        .collect::<Vec<_>>();
    assert_eq!(
        delays,
        [
            Duration::ZERO,
            Duration::from_millis(25),
            Duration::ZERO,
            Duration::ZERO
        ]
    );
}

#[test]
fn streaming_zero_delay_pacing_waits_for_nothing() {
    let steps = stream("abcdef", pacing(2, 0));
    assert!(steps.iter().all(|step| step.delay_after.is_zero()));
}

#[test]
fn streaming_events_share_ids_and_only_the_last_status_is_final() {
    let steps = stream("abcdef", pacing(2, 10));
    assert!(
        steps
            .iter()
            .all(|step| step.event.id == "req-1" && step.event.jsonrpc == "2.0")
    );
    let finals = steps
        .iter()
        .filter_map(|step| match &step.event.result {
            WxoStreamResult::Status(status) => Some(status.final_event),
            WxoStreamResult::Task(_) | WxoStreamResult::Artifact(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(finals, [false, true]);
    let artifact_ids = artifacts(&steps)
        .iter()
        .map(|result| result.artifact.artifact_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(artifact_ids, [uuid(3), uuid(3), uuid(3)]);
}

#[rstest]
#[case::two_byte_letters("\u{e9}", 60, &[50, 10])]
#[case::three_byte_letters("\u{65e5}", 51, &[50, 1])]
#[case::four_byte_emoji("\u{1f980}", 100, &[50, 50])]
fn streaming_multibyte_text_is_split_by_characters_not_bytes(
    #[case] character: &str,
    #[case] count: usize,
    #[case] expected: &[usize],
) {
    let text = character.repeat(count);
    let chunks = artifact_texts(&stream(&text, StreamPacing::default()));
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.chars().count())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(chunks.concat(), text);
}

#[test]
fn streaming_multibyte_text_never_splits_inside_a_character() {
    let chunks = artifact_texts(&stream("a\u{e9}\u{65e5}\u{1f980}b", pacing(2, 10)));
    assert_eq!(chunks, ["a\u{e9}", "\u{65e5}\u{1f980}", "b"]);
}

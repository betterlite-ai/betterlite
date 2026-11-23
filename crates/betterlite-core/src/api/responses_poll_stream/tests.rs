#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_poll_stream::frame::{PollFrame, decode_poll_stream_frame};

mod accumulator_rules;
mod cancel_failure;
mod frame_selection;
mod provider_call;
mod support;
mod terminal_merge;
mod text_flush;

#[tokio::test]
async fn polling_provider_call_survives_disconnect() {
    let scenario = provider_call::disconnected_stream().await;
    provider_call::assert_provider_call(&scenario);
    provider_call::assert_scope_preserved(&scenario);
    provider_call::assert_single_admission(&scenario);
    provider_call::assert_record_completed(&scenario);
}

#[tokio::test]
async fn polling_text_flush_order() {
    let scenario = text_flush::flushed_stream().await;
    text_flush::assert_batches(&scenario);
    text_flush::assert_clock_reads(&scenario);
    accumulator_rules::duplicate_ids_keep_insertion_order();
    accumulator_rules::missing_ids_are_ignored();
    accumulator_rules::part_done_replaces_selected_part();
    accumulator_rules::part_done_negative_index_keeps_parts();
    accumulator_rules::delta_negative_index_updates_last_part();
    accumulator_rules::delta_out_of_range_marks_dirty();
    accumulator_rules::item_done_replaces_item();
}

#[tokio::test]
async fn polling_terminal_merge() {
    terminal_merge::assert_m1_then_m2(&terminal_merge::merged_items().await);
    terminal_merge::assert_m1_then_m2(&terminal_merge::terminal_keeps_iterating().await);
    terminal_merge::assert_only_m1(&terminal_merge::empty_terminal_output().await);
    terminal_merge::assert_failure_captured(&terminal_merge::failure_terminal().await);
    terminal_merge::assert_terminal_fields(&terminal_merge::terminal_fields().await);
    let (scenario, warnings) = terminal_merge::non_streaming().await;
    terminal_merge::assert_completed_without_iterating(&scenario, &warnings);
}

#[tokio::test]
async fn polling_cancel_does_not_stop_upstream() {
    let cancelled = cancel_failure::cancelled_mid_stream().await;
    cancel_failure::assert_cancelled_then_completed(&cancelled);
    let (failed, logs) = cancel_failure::failed_processor().await;
    cancel_failure::assert_failed_record(&failed);
    cancel_failure::assert_error_logged(&logs);
}

#[tokio::test]
async fn poll_frame_literal_selection() {
    let (split, warnings) = frame_selection::split_payload().await;
    frame_selection::assert_warned(&warnings, "Failed to parse streaming chunk");
    frame_selection::assert_completed(&split);
    let (scalar, _) = frame_selection::scalar_payload().await;
    frame_selection::assert_failed_message(&scalar, "the streaming event is not a JSON object");
    let (utf8, _) = frame_selection::invalid_utf8().await;
    frame_selection::assert_failed_message(&utf8, "the provider frame is not utf-8");
    let (done, _) = frame_selection::done_before_terminal().await;
    frame_selection::assert_completed(&done);
    let (unknown, _) = frame_selection::unknown_event_flushes().await;
    frame_selection::assert_flushed_before_final(&unknown);
}

#[tokio::test]
async fn failure_messages_are_bounded_and_redacted() {
    let long = cancel_failure::long_failed_processor("x".repeat(20_000)).await;
    cancel_failure::assert_bounded(&cancel_failure::message_of(&long));
    let secret = format!(
        "boom sk-{} Traceback (most recent call last): secret tail",
        "a".repeat(30)
    );
    let redacted = cancel_failure::long_failed_processor(secret).await;
    cancel_failure::assert_redacted(&cancel_failure::message_of(&redacted));
    let source = cancel_failure::failed_chunk_source("the provider stream broke").await;
    cancel_failure::assert_failed_message(&source, "the provider stream broke");
    let (scenario, error) = cancel_failure::failed_write().await;
    cancel_failure::assert_write_error(&scenario, error);
}

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fuzz/corpus/responses_poll_stream"
);

#[test]
fn replays_corpus() {
    let seeds: Vec<PathBuf> = fs::read_dir(CORPUS)
        .expect("the corpus directory exists")
        .map(|entry| entry.expect("the corpus entry lists").path())
        .collect();
    assert!(!seeds.is_empty(), "the corpus has seeds");
    let wrong = seeds
        .iter()
        .filter(|path| {
            let bytes = fs::read(path).expect("the seed reads");
            let kind = match decode_poll_stream_frame(&bytes, ApiLimits::default()) {
                Ok(PollFrame::NoData) => "no_data",
                Ok(PollFrame::Done) => "done",
                Ok(PollFrame::Event(_)) => "event",
                Ok(PollFrame::Malformed) => "malformed",
                Err(_) => "error",
            };
            let expected = match path.file_name().and_then(|name| name.to_str()) {
                Some("delta.json" | "scalar.json" | "python-whitespace.txt") => "event",
                Some("done.txt") => "done",
                Some("no-space.txt" | "comment.txt" | "empty.bin") => "no_data",
                Some("malformed.json") => "malformed",
                Some("invalid-utf8.bin") => "error",
                Some(other) => panic!("unexpected corpus seed {other}"),
                None => panic!("the seed name is text"),
            };
            kind != expected
        })
        .count();
    assert_eq!(wrong, 0, "every seed decodes the right way");
}

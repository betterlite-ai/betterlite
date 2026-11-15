#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde::Serialize;

use super::{
    StatementLimits, spend_log_queue_within_budget, spend_log_row_bytes, spend_log_write_batches,
};

#[derive(Serialize)]
struct TestRow {
    request_id: String,
    tool_name: String,
}

#[derive(Serialize)]
struct TaggedRow {
    request_id: String,
    tags: Vec<u8>,
}

// serde_json writes integer keys as strings, so the refused row keys by a tuple, which it rejects
#[derive(Serialize)]
struct Refused {
    keys: BTreeMap<(i64, i64), i64>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Measured {
    Row(TestRow),
    Refused(Refused),
}

fn row(request_id: &str, tool_name: &str) -> TestRow {
    TestRow {
        request_id: request_id.to_owned(),
        tool_name: tool_name.to_owned(),
    }
}

fn refused() -> Measured {
    Measured::Refused(Refused {
        keys: BTreeMap::from([((1, 2), 3)]),
    })
}

fn measured(request_id: &str, tool_name: &str) -> Measured {
    Measured::Row(row(request_id, tool_name))
}

const fn limits(max_bytes: usize, max_rows: usize) -> StatementLimits {
    StatementLimits {
        max_bytes,
        max_rows,
    }
}

fn words(rows: &[String]) -> Vec<&str> {
    rows.iter().map(String::as_str).collect()
}

fn batch_lengths<'rows, Row>(batches: &[&'rows [Row]]) -> Vec<usize> {
    batches.iter().map(|batch| batch.len()).collect()
}

#[rstest]
#[case::plain("a", "b", r#"{"request_id": "a", "tool_name": "b"}"#)]
#[case::spaces(
    "a b",
    "two words",
    r#"{"request_id": "a b", "tool_name": "two words"}"#
)]
#[case::escapes("a\"b\\c", "x\ny", r#"{"request_id": "a\"b\\c", "tool_name": "x\ny"}"#)]
#[case::non_ascii("a", "\u{754c}", "{\"request_id\": \"a\", \"tool_name\": \"\\u754c\"}")]
#[case::controls(
    "\u{1}",
    "\u{7f}",
    "{\"request_id\": \"\\u0001\", \"tool_name\": \"\\u007f\"}"
)]
#[case::astral(
    "\u{1f600}",
    "\u{a0}",
    "{\"request_id\": \"\\ud83d\\ude00\", \"tool_name\": \"\\u00a0\"}"
)]
fn measures_a_row_as_python_dumps_writes_it(
    #[case] request_id: &str,
    #[case] tool_name: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        spend_log_row_bytes(&row(request_id, tool_name)),
        expected.len()
    );
}

#[test]
fn measures_an_array_with_python_separators() {
    let tagged = TaggedRow {
        request_id: String::from("a"),
        tags: vec![1, 2],
    };
    assert_eq!(
        spend_log_row_bytes(&tagged),
        r#"{"request_id": "a", "tags": [1, 2]}"#.len()
    );
}

#[test]
fn counts_the_framing_and_one_separator_per_extra_row() {
    let rows: Vec<String> = ["ab", "cd", "ef"].map(str::to_owned).to_vec();
    let fitted = spend_log_write_batches(&rows, limits(12, 100));
    assert_eq!(batch_lengths(&fitted), [2, 1]);
    let tight = spend_log_write_batches(&rows, limits(11, 100));
    assert_eq!(batch_lengths(&tight), [1, 1, 1]);
    let by_rows = spend_log_write_batches(&rows, limits(1_000, 2));
    assert_eq!(batch_lengths(&by_rows), [2, 1]);
    let by_zero_rows = spend_log_write_batches(&rows, limits(1_000, 0));
    assert_eq!(batch_lengths(&by_zero_rows), [1, 1, 1]);
    assert!(spend_log_write_batches::<String>(&[], limits(12, 100)).is_empty());
}

#[test]
fn keeps_a_row_larger_than_the_budget_alone() {
    let rows: Vec<String> = ["abcd", "efgh"].map(str::to_owned).to_vec();
    let batches = spend_log_write_batches(&rows, limits(3, 100));
    assert_eq!(batch_lengths(&batches), [1, 1]);
    assert_eq!(
        words(batches.first().copied().unwrap_or_default()),
        ["abcd"]
    );
}

#[test]
fn slices_cover_every_row_once_in_order() {
    let rows: Vec<String> = ["a", "bb", "ccc", "dddd"].map(str::to_owned).to_vec();
    let batches = spend_log_write_batches(&rows, limits(16, 100));
    assert_eq!(batch_lengths(&batches), [2, 2]);
    let flattened: Vec<&String> = batches.iter().flat_map(|batch| batch.iter()).collect();
    assert_eq!(flattened, rows.iter().collect::<Vec<_>>());
}

#[test]
fn an_unmeasurable_row_measures_zero_and_still_ships() {
    let rows = [refused(), measured("a", "b")];
    assert_eq!(spend_log_row_bytes(&rows[0]), 0);
    assert_eq!(spend_log_row_bytes(&rows[1]), 37);
    let together = spend_log_write_batches(&rows, limits(41, 100));
    assert_eq!(batch_lengths(&together), [2]);
    let apart = spend_log_write_batches(&rows, limits(40, 100));
    assert_eq!(batch_lengths(&apart), [1, 1]);
}

#[test]
fn keeps_every_row_while_the_queue_fits() {
    let rows: Vec<String> = ["ab", "cd"].map(str::to_owned).to_vec();
    let trimmed = spend_log_queue_within_budget(&rows, 12, 16);
    assert_eq!(trimmed.rows, rows.as_slice());
    assert_eq!(trimmed.queued_bytes, 12);
    let single = spend_log_queue_within_budget(&rows[..1], 99, 1);
    assert_eq!(single.rows.len(), 1);
    assert_eq!(single.queued_bytes, 99);
    let fitting_single = spend_log_queue_within_budget(&rows[..1], 3, 16);
    assert_eq!(fitting_single.rows.len(), 1);
    assert_eq!(fitting_single.queued_bytes, 3);
}

#[test]
fn drops_the_oldest_rows_until_the_running_total_fits() {
    let rows: Vec<String> = ["a1", "b2", "c3", "d4", "e5"].map(str::to_owned).to_vec();
    let trimmed = spend_log_queue_within_budget(&rows, 20, 12);
    assert_eq!(words(trimmed.rows), ["c3", "d4", "e5"]);
    assert_eq!(trimmed.queued_bytes, 12);
}

#[test]
fn keeps_the_newest_row_when_it_alone_exceeds_the_budget() {
    let rows: Vec<String> = ["a1", "b2"].map(str::to_owned).to_vec();
    let trimmed = spend_log_queue_within_budget(&rows, 8, 3);
    assert_eq!(words(trimmed.rows), ["b2"]);
    assert_eq!(trimmed.queued_bytes, 4);
}

#[test]
fn a_row_that_cannot_be_measured_frees_no_budget() {
    let rows = [refused(), measured("a", "b")];
    let trimmed = spend_log_queue_within_budget(&rows, 10, 5);
    assert_eq!(trimmed.rows.len(), 1);
    assert_eq!(trimmed.queued_bytes, spend_log_row_bytes(&rows[1]));
}

#[test]
fn a_running_total_that_overshoots_the_dropped_rows_stops_at_zero() {
    let rows: Vec<String> = ["a1", "b2", "c3"].map(str::to_owned).to_vec();
    let trimmed = spend_log_queue_within_budget(&rows, 2, 1);
    assert_eq!(words(trimmed.rows), ["b2", "c3"]);
    assert_eq!(trimmed.queued_bytes, 0);
}
